//! Loopback only. The path policy lives in [`http`].
use super::{
    http::{self, Body},
    proxy::{Rule, Rules},
};
use crate::{
    commands::dev::Options,
    context::Context,
    error::{Error, Result},
    layout,
    pipeline::{manifest::OutputManifest, publish, site::route},
    reporter::elapsed_text,
    workspace::Project,
};
use http_body_util::{BodyExt, Full, combinators::UnsyncBoxBody};
use hyper::{
    Method, Request,
    body::{Bytes, Incoming},
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::time::Instant;
use std::{
    convert::Infallible,
    io,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener},
    rc::Rc,
    sync::{Arc, RwLock},
    task::Poll,
};

/// One application in a dev session: the settings it builds with, and the
/// project state its watcher keeps current.
pub(crate) struct DevApp {
    pub cx: Context,
    pub project: Project,
}

/// Serve every app from one origin, each under its own base path, so links
/// between them work as they will when deployed.
pub(crate) fn dev(
    cx: &Context,
    apps: Vec<DevApp>,
    options: &Options,
    started: Instant,
    proxy: Rules,
) -> Result {
    validate_apps(&apps)?;

    let listeners = bind(options.port)?;
    let mut routes = Vec::new();
    let mut watchers = Vec::new();
    for DevApp { cx, project } in apps {
        let base = project.config.base_path.clone();
        let state = Arc::new(RwLock::new(project.clone()));
        watchers.push(super::watch::start(&cx, project, state.clone())?);
        routes.push((base, (cx, state)));
    }
    let base = routes
        .iter()
        .map(|(base, _)| base.as_str())
        .min_by_key(|base| base.len())
        .unwrap_or("/")
        .to_owned();
    ready(
        cx,
        options.port,
        &base,
        options.open,
        &format!(
            "Ready in {}, watching for changes. Press Ctrl+C to stop.",
            elapsed_text(started.elapsed())
        ),
    )?;
    cx.reporter.blank();

    serve(cx, listeners, proxy, move |method, url, accept| {
        let Some((cx, state)) = route(&routes, url) else {
            return http::build_response(404, "text/plain", b"Not found".to_vec(), false);
        };
        respond_project(cx, state, method, url, accept)
    })
}

fn respond_project(
    cx: &Context,
    state: &RwLock<Project>,
    method: &Method,
    url: &str,
    accept: &str,
) -> Body {
    // A publication must not swap the directory out from under a read.
    let Ok(_access) = publish::OUTPUT_ACCESS.lock() else {
        return http::error_response("the output publication lock was poisoned");
    };
    let Ok(project) = state.read() else {
        return http::error_response("the preview state lock was poisoned");
    };
    http::respond(
        method,
        url,
        accept,
        http::Route {
            directory: &project.output(cx),
            base: &project.config.base_path,
            history_fallback: &project.config.history_fallback,
        },
        http::Serving::Development,
    )
}

fn validate_apps(apps: &[DevApp]) -> Result {
    for DevApp { cx, project } in apps {
        project.validate_output(cx)?;
        let output = OutputManifest::read(&project.output(cx))?;
        // Patching a document whose URLs moved would leave its links broken.
        if output.base_path != project.config.base_path {
            return Err(Error::project("base-path changed since the last build")
                .remedy("run `fusor build`"));
        }
        if output.history_fallback != project.config.history_fallback {
            return Err(
                Error::project("history-fallback changed since the last build")
                    .remedy("run `fusor build`"),
            );
        }
    }
    Ok(())
}

pub(crate) fn check_port(port: u16) -> Result {
    bind(port).map(drop)
}

/// Both loopback addresses, since `localhost` may resolve to either.
pub(crate) fn bind(port: u16) -> Result<Vec<TcpListener>> {
    let ipv4 = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let ipv6 = SocketAddr::from((Ipv6Addr::LOCALHOST, port));
    let ipv4_listener = TcpListener::bind(ipv4).map_err(|error| cannot_listen(ipv4, error))?;
    match TcpListener::bind(ipv6) {
        Ok(ipv6_listener) => Ok(vec![ipv4_listener, ipv6_listener]),
        // Another server there would answer `localhost` in place of this one.
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => Err(cannot_listen(ipv6, error)),
        // A host without IPv6 has no `::1`, so its `localhost` is 127.0.0.1.
        Err(_) => Ok(vec![ipv4_listener]),
    }
}

fn cannot_listen(address: SocketAddr, error: io::Error) -> Error {
    Error::project(format!("cannot listen on {address}: {error}"))
        .remedy("stop the process using that port, or pass --port PORT")
}

/// `localhost`, not 127.0.0.1: browsers treat them as different sites, and
/// passkeys reject an IP address.
pub(crate) fn url(port: u16, base: &str) -> String {
    format!("http://localhost:{port}{base}")
}

/// The server is listening: say so, give scripts the address, and open it.
pub(crate) fn ready(cx: &Context, port: u16, base: &str, open: bool, message: &str) -> Result {
    let url = url(port, base);
    cx.reporter.done(message);
    cx.reporter.address(&url)?;
    if open {
        open_browser(&url)?;
    }
    Ok(())
}

pub(crate) fn serve(
    cx: &Context,
    listeners: Vec<TcpListener>,
    proxy: Rules,
    answer: impl Fn(&Method, &str, &str) -> Body + 'static,
) -> Result {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let cx = Rc::new(cx.clone());
    let answer = Rc::new(answer);
    let proxy = Rc::new(proxy);
    // Keep idle and speculative browser connections off a blocking worker pool:
    // a burst must not strand a script request behind another keep-alive socket.
    tokio::task::LocalSet::new().block_on(&runtime, async move {
        let listeners = listeners
            .into_iter()
            .map(|listener| {
                listener.set_nonblocking(true)?;
                tokio::net::TcpListener::from_std(listener)
            })
            .collect::<io::Result<Vec<_>>>()?;
        loop {
            let stream = accept(&listeners).await?;
            tokio::task::spawn_local(serve_connection(
                cx.clone(),
                stream,
                proxy.clone(),
                answer.clone(),
            ));
        }
    })
}

/// The next connection on any of the listeners.
async fn accept(listeners: &[tokio::net::TcpListener]) -> io::Result<tokio::net::TcpStream> {
    let (stream, _) = std::future::poll_fn(|context| {
        listeners
            .iter()
            .map(|listener| listener.poll_accept(context))
            .find(Poll::is_ready)
            .unwrap_or(Poll::Pending)
    })
    .await?;
    Ok(stream)
}

async fn serve_connection(
    cx: Rc<Context>,
    stream: tokio::net::TcpStream,
    proxy: Rc<Rules>,
    answer: Rc<impl Fn(&Method, &str, &str) -> Body>,
) {
    let service = service_fn(|request| async {
        Ok::<_, Infallible>(answer_request(&cx, &proxy, &*answer, request).await)
    });
    if let Err(error) = hyper::server::conn::http1::Builder::new()
        .timer(TokioTimer::new())
        .serve_connection(TokioIo::new(stream), service)
        .await
    {
        cx.reporter.note(format!("HTTP: {error}"));
    }
}

type ServerResponse = hyper::Response<UnsyncBoxBody<Bytes, hyper::Error>>;

async fn answer_request(
    cx: &Context,
    proxy: &Rules,
    answer: &impl Fn(&Method, &str, &str) -> Body,
    request: Request<Incoming>,
) -> ServerResponse {
    let started = Instant::now();
    let method = request.method().clone();
    let uri = request.uri().clone();
    let url = uri.path_and_query().map_or("/", |value| value.as_str());
    let response = match proxy.matching(uri.path()) {
        Some(rule) => forward(cx, rule, request).await,
        None => {
            // A non-text Accept header cannot authorize the HTML history fallback.
            let accept = request
                .headers()
                .get("Accept")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default();
            full_response(answer(&method, url, accept))
        }
    };
    let status = response.status().as_u16();
    if logged(cx, url, &response, status) {
        cx.reporter
            .request(method.as_str(), uri.path(), status, started.elapsed());
    }
    response
}

/// A backend that cannot answer is a bad gateway, reported in the terminal too.
async fn forward(cx: &Context, rule: &Rule, request: Request<Incoming>) -> ServerResponse {
    match rule.forward(request, cx.reporter.clone()).await {
        Ok(response) => response.map(BodyExt::boxed_unsync),
        Err(error) => {
            cx.reporter.note(format!("Proxy: {error}"));
            full_response(http::build_response(
                502,
                "text/plain",
                error.to_string().into_bytes(),
                false,
            ))
        }
    }
}

fn full_response(response: Body) -> ServerResponse {
    response.map(|body| {
        Full::new(Bytes::from(body))
            .map_err(|never| match never {})
            .boxed_unsync()
    })
}

/// Page loads and errors, like a framework dev server: not every script, image
/// and stylesheet a page pulls in, and never the refresh client's polling.
fn logged(cx: &Context, url: &str, response: &ServerResponse, status: u16) -> bool {
    let path = url.split('?').next().unwrap_or("");
    if ["version", "update"]
        .iter()
        .any(|name| path.ends_with(&format!("{}/{name}", layout::GENERATED)))
    {
        return false;
    }
    let document = response
        .headers()
        .get("Content-Type")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"));
    document || status >= 400 || cx.reporter.is_verbose()
}

fn open_browser(url: &str) -> Result {
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(target_os = "windows") {
        let mut command = std::process::Command::new("rundll32");
        command.arg("url.dll,FileProtocolHandler");
        command
    } else {
        std::process::Command::new("xdg-open")
    };
    command.arg(url).spawn()?;
    Ok(())
}
