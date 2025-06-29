use rustls::ClientConfig;
use rustls::RootCertStore;
use rustls::ServerConfig;
use rustls::crypto::aws_lc_rs as provider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct TlsConfig {
    pub server_config: Arc<ServerConfig>,
    pub client_config: Arc<ClientConfig>,
}

impl TlsConfig {
    pub fn new(
        cert_path: &str,
        key_path: &str,
        ca_path: Option<&str>,
    ) -> anyhow::Result<TlsConfig> {
        let certs = Self::load_certificates(cert_path)?;
        let key = Self::load_private_key(key_path)?;

        let mut root_store = RootCertStore::empty();
        if let Some(ca_path) = ca_path {
            root_store.add_parsable_certificates(
                CertificateDer::pem_file_iter(ca_path)?.map(|result| result.unwrap()),
            );
        } else {
            root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }

        let mut server = ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;

        server.alpn_protocols = vec!["http/1.1".into()];

        let mut client = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        client.alpn_protocols = vec!["http/1.1".into()];

        client.dangerous().set_certificate_verifier(Arc::new(
            danger::NoCertificateVerification::new(provider::default_provider()),
        ));

        Ok(TlsConfig {
            server_config: Arc::new(server),
            client_config: Arc::new(client),
        })
    }

    pub fn load_certificates(path: &str) -> anyhow::Result<Vec<CertificateDer<'static>>> {
        Ok(CertificateDer::pem_file_iter(path)?
            .map(|result| result)
            .collect::<Result<Vec<_>, _>>()?)
    }

    pub fn load_private_key(path: &str) -> anyhow::Result<PrivateKeyDer<'static>> {
        Ok(PrivateKeyDer::from_pem_file(path)?)
    }
}

mod danger {
    use rustls::DigitallySignedStruct;
    use rustls::client::danger::HandshakeSignatureValid;
    use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};

    #[derive(Debug)]
    pub struct NoCertificateVerification(CryptoProvider);

    impl NoCertificateVerification {
        pub fn new(provider: CryptoProvider) -> Self {
            Self(provider)
        }
    }

    impl rustls::client::danger::ServerCertVerifier for NoCertificateVerification {
        fn verify_server_cert(
            &self,
            _end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &ServerName<'_>,
            _ocsp: &[u8],
            _now: UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls12_signature(
                message,
                cert,
                dss,
                &self.0.signature_verification_algorithms,
            )
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            verify_tls13_signature(
                message,
                cert,
                dss,
                &self.0.signature_verification_algorithms,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            self.0.signature_verification_algorithms.supported_schemes()
        }
    }
}
