use std::{env, fs, path::PathBuf};

use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

fn main() -> std::io::Result<()> {
    let path = env::current_dir()?;

    create_cert(path.join("client"))?;
    create_cert(path.join("server"))?;

    Ok(())
}

fn create_cert(save_base_path: PathBuf) -> std::io::Result<()> {
    let cert_path = save_base_path.join("cert.der");
    let key_path = save_base_path.join("key.der");

    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let key = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
    let cert: CertificateDer = cert.cert.into();

    fs::create_dir_all(save_base_path)?;
    fs::write(&cert_path, &cert)?;
    fs::write(&key_path, key.secret_pkcs8_der())?;

    Ok(())
}
