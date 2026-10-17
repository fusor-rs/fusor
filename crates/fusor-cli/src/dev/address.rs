//! Where a server listens, and the URL it prints and opens.
use crate::{
    error::{Error, Result},
    workspace::Project,
};
use fusor_build::app::DevConfig;
use std::{
    fmt, io,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener},
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        self, InconsistentKeys, ServerConfig,
        crypto::ring,
        pki_types::{
            CertificateDer, PrivateKeyDer,
            pem::{self, PemObject},
        },
    },
};

const HTTPS_PORT: u16 = 443;

/// A PEM certificate chain and its private key.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TlsFiles {
    pub cert: PathBuf,
    pub key: PathBuf,
}

/// The `fusor dev` options that `[package.metadata.fusor.dev]` also accepts.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AddressConfig {
    /// `None` listens on both loopback addresses.
    pub host: Option<IpAddr>,
    /// The host name printed and opened in place of the address.
    pub name: Option<String>,
    pub tls: Option<TlsFiles>,
}

impl AddressConfig {
    /// Each option given on the command line replaces the configured one. Every
    /// application served together must configure the same address.
    pub(crate) fn with_projects<'a>(
        self,
        projects: impl IntoIterator<Item = &'a Project>,
    ) -> Result<Self> {
        let mut configured: Option<(&str, Self)> = None;
        for project in projects {
            let config = Self::from_metadata(&project.config.dev, &project.root)
                .map_err(|error| error.context(&project.name))?;
            if config == Self::default() {
                continue;
            }
            if let Some((other, _)) = configured
                .as_ref()
                .filter(|(_, previous)| *previous != config)
            {
                return Err(Error::project(format!(
                    "{other} and {} configure different development addresses",
                    project.name
                ))
                .remedy("set the same host, name, tls-cert and tls-key in [package.metadata.fusor.dev] of both, or pass them as options"));
            }
            configured = Some((&project.name, config));
        }
        let configured = configured.map(|(_, config)| config).unwrap_or_default();
        Ok(Self {
            host: self.host.or(configured.host),
            name: self.name.or(configured.name),
            tls: self.tls.or(configured.tls),
        })
    }

    /// Files are relative to the package, as other paths in its metadata are.
    fn from_metadata(dev: &DevConfig, root: &Path) -> Result<Self> {
        let tls = match (&dev.tls_cert, &dev.tls_key) {
            (Some(cert), Some(key)) => Some(TlsFiles {
                cert: root.join(cert),
                key: root.join(key),
            }),
            (None, None) => None,
            _ => {
                return Err(Error::project(
                    "[package.metadata.fusor.dev] sets only one of tls-cert and tls-key",
                )
                .remedy("set both tls-cert and tls-key, or neither"));
            }
        };
        Ok(Self {
            host: dev.host,
            name: dev.name.clone(),
            tls,
        })
    }
}

pub(crate) struct Address {
    host: Option<IpAddr>,
    name: Option<String>,
    port: u16,
    pub tls: Option<TlsAcceptor>,
}

impl Address {
    pub(crate) fn loopback(port: u16) -> Self {
        Self {
            host: None,
            name: None,
            port,
            tls: None,
        }
    }

    /// Reads the certificate and key now, so a bad file fails before a build.
    pub(crate) fn new(config: AddressConfig, port: u16) -> Result<Self> {
        Ok(Self {
            host: config.host,
            name: config.name,
            port,
            tls: config.tls.as_ref().map(acceptor).transpose()?,
        })
    }

    /// The host when one is given; otherwise both loopback addresses, since
    /// `localhost` may resolve to either.
    pub(crate) fn bind(&self) -> Result<Vec<TcpListener>> {
        if let Some(host) = self.host {
            let address = SocketAddr::new(host, self.port);
            let listener = TcpListener::bind(address).map_err(|error| {
                let remedy = if error.kind() == io::ErrorKind::PermissionDenied {
                    "pass --port 1024 or above; macOS allows any port with --host 0.0.0.0"
                } else {
                    "pass --host with an address of this machine, or --port PORT"
                };
                cannot_listen(address, error).remedy(remedy)
            })?;
            return Ok(vec![listener]);
        }
        let ipv4 = SocketAddr::from((Ipv4Addr::LOCALHOST, self.port));
        let ipv6 = SocketAddr::from((Ipv6Addr::LOCALHOST, self.port));
        let port_in_use = "stop the process using that port, or pass --port PORT";
        let ipv4_listener = TcpListener::bind(ipv4)
            .map_err(|error| cannot_listen(ipv4, error).remedy(port_in_use))?;
        match TcpListener::bind(ipv6) {
            Ok(ipv6_listener) => Ok(vec![ipv4_listener, ipv6_listener]),
            // Another server there would answer `localhost` in place of this one.
            Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                Err(cannot_listen(ipv6, error).remedy(port_in_use))
            }
            // A host without IPv6 has no `::1`, so its `localhost` is 127.0.0.1.
            Err(_) => Ok(vec![ipv4_listener]),
        }
    }

    /// `localhost`, not 127.0.0.1, for loopback: browsers treat them as
    /// different sites, and passkeys reject an IP address.
    pub(crate) fn url(&self, base: &str) -> String {
        let scheme = if self.tls.is_some() { "https" } else { "http" };
        let host = match (&self.name, self.host) {
            (Some(name), _) => name.clone(),
            (None, Some(IpAddr::V4(address))) if !address.is_unspecified() => address.to_string(),
            (None, Some(IpAddr::V6(address))) if !address.is_unspecified() => {
                format!("[{address}]")
            }
            (None, _) => "localhost".to_owned(),
        };
        if self.tls.is_some() && self.port == HTTPS_PORT {
            return format!("{scheme}://{host}{base}");
        }
        format!("{scheme}://{host}:{}{base}", self.port)
    }
}

fn cannot_listen(address: SocketAddr, error: io::Error) -> Error {
    Error::project(format!("cannot listen on {address}: {error}"))
}

fn acceptor(files: &TlsFiles) -> Result<TlsAcceptor> {
    let certificates = CertificateDer::pem_file_iter(&files.cert)
        .and_then(|sections| sections.collect::<std::result::Result<Vec<_>, _>>())
        .map_err(|error| unreadable("a certificate", &files.cert, error))?;
    if certificates.is_empty() {
        return Err(unreadable(
            "a certificate",
            &files.cert,
            pem::Error::NoItemsFound,
        ));
    }
    let key = PrivateKeyDer::from_pem_file(&files.key)
        .map_err(|error| unreadable("a private key", &files.key, error))?;
    let config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .and_then(|builder| {
            builder
                .with_no_client_auth()
                .with_single_cert(certificates, key)
        })
        .map_err(|error| {
            let (key, cert) = (files.key.display(), files.cert.display());
            let message = match error {
                rustls::Error::InconsistentKeys(InconsistentKeys::KeyMismatch) => {
                    format!("the key in {key} does not match the certificate in {cert}")
                }
                error => {
                    format!("cannot use the key in {key} with the certificate in {cert}: {error}")
                }
            };
            Error::project(message)
                .remedy("pass the private key of the first certificate in the chain")
        })?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

fn unreadable(item: &str, path: &Path, error: impl fmt::Display) -> Error {
    Error::project(format!(
        "cannot read {item} from {}: {error}",
        path.display()
    ))
    .remedy("pass PEM files with --tls-cert and --tls-key, or in [package.metadata.fusor.dev]")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn project(name: &str, dev: &str) -> Project {
        let root = PathBuf::from("/apps").join(name);
        Project {
            id: name.into(),
            name: name.into(),
            manifest: root.join("Cargo.toml"),
            workspace: "/apps".into(),
            target: "/apps/target".into(),
            watch_roots: vec![root.clone()],
            config: toml::from_str(&format!("dev = {{ {dev} }}")).unwrap(),
            root,
        }
    }

    #[test]
    fn options_replace_the_configuration_that_served_applications_share() {
        let configured =
            r#"host = "0.0.0.0", name = "shell.local", tls-cert = "cert.pem", tls-key = "key.pem""#;
        let options = AddressConfig {
            name: Some("dev.example.local".into()),
            ..AddressConfig::default()
        };
        let address = options
            .with_projects(&[project("shell", configured), project("docs", "")])
            .unwrap();
        assert_eq!(
            address,
            AddressConfig {
                host: Some(Ipv4Addr::UNSPECIFIED.into()),
                name: Some("dev.example.local".into()),
                tls: Some(TlsFiles {
                    cert: "/apps/shell/cert.pem".into(),
                    key: "/apps/shell/key.pem".into(),
                }),
            }
        );

        for (projects, message) in [
            (
                vec![project("shell", r#"tls-cert = "cert.pem""#)],
                "shell: [package.metadata.fusor.dev] sets only one of tls-cert and tls-key",
            ),
            (
                vec![
                    project("shell", r#"host = "0.0.0.0""#),
                    project("docs", r#"host = "127.0.0.1""#),
                ],
                "shell and docs configure different development addresses",
            ),
        ] {
            let error = AddressConfig::default()
                .with_projects(&projects)
                .unwrap_err();
            assert_eq!(error.to_string().lines().next(), Some(message));
        }
    }
}
