//! https pour la page de l'iPad : Safari ne donne l'inclinaison et les secousses de l'appareil
//! qu'aux pages sécurisées. Certificat fabriqué ici (auto-signé, accepté une fois sur l'iPad),
//! gardé dans le dossier de l'app et refait quand l'adresse de l'ordinateur change ou qu'il
//! approche de sa fin de validité.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// Durée de validité : Apple refuse les certificats de serveur de plus de 825 jours.
const VALID_DAYS: i64 = 800;
/// Refait au-delà (marge avant la fin de validité).
const RENEW_DAYS: u64 = 700;

/// Configuration du serveur https (certificat chargé ou fabriqué pour ces adresses).
pub fn server_config(dir: &Path, names: &[String]) -> Result<Arc<rustls::ServerConfig>> {
    let (cert, key) = certificate(dir, names)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)?;
    Ok(Arc::new(config))
}

/// Certificat gardé s'il couvre ces adresses et n'est pas trop vieux ; sinon un nouveau.
fn certificate(dir: &Path, names: &[String]) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    let (cert_path, key_path, names_path) = (dir.join("ipad-cert.der"), dir.join("ipad-key.der"), dir.join("ipad-cert.txt"));
    let fresh = std::fs::metadata(&cert_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < Duration::from_secs(RENEW_DAYS * 86_400));
    let known: Vec<String> = std::fs::read_to_string(&names_path).unwrap_or_default().lines().map(str::to_owned).collect();
    if fresh
        && names.iter().all(|n| known.contains(n))
        && let (Ok(cert), Ok(key)) = (std::fs::read(&cert_path), std::fs::read(&key_path))
    {
        return Ok((CertificateDer::from(cert), PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key))));
    }
    // Nouveau certificat : les adresses d'avant restent couvertes (l'iPad garde son accord).
    let mut all = known;
    for n in names {
        if !all.contains(n) {
            all.push(n.clone());
        }
    }
    let (cert, key) = make_certificate(&all)?;
    std::fs::create_dir_all(dir).ok();
    std::fs::write(&cert_path, &cert).ok();
    std::fs::write(&key_path, &key).ok();
    std::fs::write(&names_path, all.join("\n")).ok();
    Ok((CertificateDer::from(cert), PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key))))
}

/// Certificat auto-signé (DER) et sa clé (PKCS#8 DER) pour ces noms et adresses.
fn make_certificate(names: &[String]) -> Result<(Vec<u8>, Vec<u8>)> {
    let key = rcgen::KeyPair::generate().context("clé du certificat")?;
    let mut params = rcgen::CertificateParams::new(names.to_vec()).context("adresses du certificat")?;
    params.distinguished_name.push(rcgen::DnType::CommonName, "OZ Mapping");
    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
    let today = days_since_epoch();
    let ((y0, m0, d0), (y1, m1, d1)) = (civil(today - 1), civil(today + VALID_DAYS));
    params.not_before = rcgen::date_time_ymd(y0, m0, d0);
    params.not_after = rcgen::date_time_ymd(y1, m1, d1);
    let cert = params.self_signed(&key).context("certificat")?;
    Ok((cert.der().to_vec(), key.serialize_der()))
}

fn days_since_epoch() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64 / 86_400)
}

/// Année, mois, jour du jour `z` après le 1er janvier 1970 (algorithme de H. Hinnant).
fn civil(z: i64) -> (i32, u8, u8) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y as i32, m as u8, d as u8)
}

/// Connexion d'une page : en clair (http) ou chiffrée (https), reconnue à son premier octet.
pub enum Conn {
    Plain(TcpStream),
    Tls(Box<rustls::StreamOwned<rustls::ServerConnection, TcpStream>>),
}

impl Conn {
    /// Ouvre la connexion : https si la page commence par une poignée de main TLS.
    pub fn accept(stream: TcpStream, tls: Option<&Arc<rustls::ServerConfig>>) -> Result<Self> {
        let mut first = [0u8; 1];
        let n = stream.peek(&mut first)?;
        match tls {
            // 0x16 : début d'une poignée de main TLS.
            Some(config) if n == 1 && first[0] == 0x16 => {
                let conn = rustls::ServerConnection::new(config.clone())?;
                Ok(Conn::Tls(Box::new(rustls::StreamOwned::new(conn, stream))))
            }
            _ => Ok(Conn::Plain(stream)),
        }
    }

    fn socket(&self) -> &TcpStream {
        match self {
            Conn::Plain(s) => s,
            Conn::Tls(s) => &s.sock,
        }
    }

    pub fn set_read_timeout(&self, d: Option<Duration>) -> std::io::Result<()> {
        self.socket().set_read_timeout(d)
    }

    #[cfg(test)]
    pub fn is_tls(&self) -> bool {
        matches!(self, Conn::Tls(_))
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.read(buf),
            Conn::Tls(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(s) => s.write(buf),
            Conn::Tls(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Plain(s) => s.flush(),
            Conn::Tls(s) => s.flush(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_723), (2024, 1, 1));
        assert_eq!(civil(20_515), (2026, 3, 3));
    }

    #[test]
    fn https_and_http_on_the_same_port() {
        let dir = std::env::temp_dir().join(format!("ozmapping-tls-{}", std::process::id()));
        let names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
        let config = server_config(&dir, &names).unwrap();
        // Gardé ensuite : le même certificat au deuxième appel.
        let first = std::fs::read(dir.join("ipad-cert.der")).unwrap();
        server_config(&dir, &names).unwrap();
        assert_eq!(std::fs::read(dir.join("ipad-cert.der")).unwrap(), first);

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (s, _) = listener.accept().unwrap();
                let mut c = Conn::accept(s, Some(&config)).unwrap();
                let mut buf = [0u8; 5];
                c.read_exact(&mut buf).unwrap();
                c.write_all(if c.is_tls() { b"tls" } else { b"txt" }).unwrap();
                c.flush().unwrap();
            }
        });
        // En clair.
        let mut plain = TcpStream::connect(addr).unwrap();
        plain.write_all(b"hello").unwrap();
        let mut got = [0u8; 3];
        plain.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"txt");
        // Chiffré, en acceptant ce certificat-là.
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from(first)).unwrap();
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let client = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let conn = rustls::ClientConnection::new(Arc::new(client), "localhost".try_into().unwrap()).unwrap();
        let mut tls = rustls::StreamOwned::new(conn, TcpStream::connect(addr).unwrap());
        tls.write_all(b"hello").unwrap();
        tls.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"tls");
        server.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
