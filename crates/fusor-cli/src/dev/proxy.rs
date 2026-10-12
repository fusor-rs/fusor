//! Development-only HTTP forwarding with streaming bodies.
use crate::{
    error::{Error, Result},
    layout,
    reporter::Reporter,
    workspace::Project,
};
use hyper::{
    HeaderMap, Request, Response, Uri,
    body::Incoming,
    header::{
        CONNECTION, HOST, HeaderName, HeaderValue, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION, TE,
        TRAILER, TRANSFER_ENCODING, UPGRADE,
    },
};
use hyper_util::rt::TokioIo;
use std::{collections::BTreeMap, str::FromStr};

const HTTP_PORT: u16 = 80;
const HOP_HEADERS: [HeaderName; 8] = [
    CONNECTION,
    HeaderName::from_static("keep-alive"),
    PROXY_AUTHENTICATE,
    PROXY_AUTHORIZATION,
    TE,
    TRAILER,
    TRANSFER_ENCODING,
    UPGRADE,
];

#[derive(Clone, Debug)]
pub(crate) struct Rule {
    prefix: String,
    address: String,
    host: HeaderValue,
}

impl Rule {
    fn parse(prefix: &str, backend: &str) -> Result<Self> {
        let path: Uri = prefix
            .parse()
            .map_err(|_| invalid("invalid proxy prefix"))?;
        if !prefix.starts_with('/')
            || prefix.contains("//")
            || path.path() != prefix
            || prefix
                .split('/')
                .any(|part| matches!(part, "." | "..") || part == layout::GENERATED)
        {
            return Err(invalid(
                "proxy prefixes must be URL paths outside the generated namespace",
            ));
        }
        let target: Uri = backend
            .parse()
            .map_err(|_| invalid("invalid proxy backend URL"))?;
        let authority = target
            .authority()
            .ok_or_else(|| invalid("proxy backend needs a host"))?;
        if target.scheme_str() != Some("http")
            || authority.host().is_empty()
            || authority.as_str().contains('@')
            || target.path() != "/"
            || target.query().is_some()
        {
            return Err(invalid(
                "proxy backend must be an HTTP origin without a path, query or credentials",
            ));
        }
        let port = match authority.port_u16() {
            Some(port) if port != 0 => port,
            None if authority.as_str() == authority.host() => HTTP_PORT,
            _ => return Err(invalid("proxy backend has an invalid port")),
        };
        Ok(Self {
            prefix: if prefix == "/" {
                prefix.to_owned()
            } else {
                prefix.trim_end_matches('/').to_owned()
            },
            address: format!("{}:{port}", authority.host()),
            host: HeaderValue::from_str(authority.as_str())
                .map_err(|_| invalid("invalid proxy backend host"))?,
        })
    }

    pub(crate) async fn forward(
        &self,
        mut request: Request<Incoming>,
        reporter: Reporter,
    ) -> Result<Response<Incoming>> {
        strip_connection_headers(request.headers_mut());
        request.headers_mut().insert(HOST, self.host.clone());
        *request.uri_mut() = request
            .uri()
            .path_and_query()
            .cloned()
            .ok_or_else(|| Error::project("proxy request needs a URL path"))?
            .into();
        let stream = tokio::net::TcpStream::connect(&self.address).await?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|error| Error::project(format!("proxy handshake failed: {error}")))?;
        tokio::task::spawn_local(async move {
            if let Err(error) = connection.await {
                reporter.note(format!("Proxy HTTP: {error}"));
            }
        });
        let mut response = sender
            .send_request(request)
            .await
            .map_err(|error| Error::project(format!("proxy request failed: {error}")))?;
        strip_connection_headers(response.headers_mut());
        Ok(response)
    }
}

impl FromStr for Rule {
    type Err = String;
    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let (prefix, backend) = value
            .split_once('=')
            .ok_or_else(|| invalid("proxy rule needs PREFIX=URL").to_string())?;
        Self::parse(prefix, backend).map_err(|error| error.to_string())
    }
}

#[derive(Default)]
pub(crate) struct Rules(Vec<Rule>);

impl Rules {
    pub(crate) fn new<'a>(
        projects: impl IntoIterator<Item = &'a Project>,
        overrides: &[Rule],
    ) -> Result<Self> {
        let mut rules: BTreeMap<String, Rule> = overrides
            .iter()
            .map(|rule| (rule.prefix.clone(), rule.clone()))
            .collect();
        for project in projects {
            for (prefix, backend) in &project.config.dev.proxy {
                let rule = Rule::parse(prefix, backend)
                    .map_err(|error| error.context(format!("{} proxy {prefix}", project.name)))?;
                if overrides.iter().any(|forced| forced.prefix == rule.prefix) {
                    continue;
                }
                if rules
                    .get(&rule.prefix)
                    .is_some_and(|previous| previous.address != rule.address)
                {
                    return Err(invalid(format!(
                        "conflicting proxy backends for {}",
                        rule.prefix
                    )));
                }
                rules.insert(rule.prefix.clone(), rule);
            }
        }
        let mut rules: Vec<_> = rules.into_values().collect();
        rules.sort_by_key(|rule| std::cmp::Reverse(rule.prefix.len()));
        Ok(Self(rules))
    }

    pub(crate) fn matching(&self, path: &str) -> Option<&Rule> {
        if path.split('/').any(|part| part == layout::GENERATED) {
            return None;
        }
        self.0.iter().find(|rule| {
            rule.prefix == "/"
                || path == rule.prefix
                || path
                    .strip_prefix(&rule.prefix)
                    .is_some_and(|tail| tail.starts_with('/'))
        })
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::project(message)
        .remedy("use --proxy /api=http://127.0.0.1:8080 or [package.metadata.fusor.dev].proxy")
}

fn strip_connection_headers(headers: &mut HeaderMap) {
    // A token that is not a valid header name cannot name a header to remove.
    let listed: Vec<HeaderName> = headers
        .get_all(CONNECTION)
        .iter()
        .flat_map(|value| value.as_bytes().split(|byte| *byte == b','))
        .filter_map(|name| HeaderName::from_bytes(name.trim_ascii()).ok())
        .collect();
    for name in listed.into_iter().chain(HOP_HEADERS) {
        headers.remove(name);
    }
}
