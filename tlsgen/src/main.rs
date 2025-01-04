use std::{env, fs};

use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

fn main() -> std::io::Result<()> {
    let path = env::current_dir()?;

    let cert_path = path.join("cert.der");
    let key_path = path.join("key.der");

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let key = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
    let cert: CertificateDer = cert.cert.into();

    fs::create_dir_all(path)?;
    fs::write(&cert_path, &cert)?;
    fs::write(&key_path, key.secret_pkcs8_der())?;

    Ok(())
}
