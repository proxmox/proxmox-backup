use failure::*;

//  chacha20-poly1305

fn rate_test(name: &str, bench: &dyn Fn() -> usize) {

    let start = std::time::SystemTime::now();
    let duration = std::time::Duration::new(1, 0);

    let mut bytes = 0;

    loop {
        bytes += bench();
        let elapsed = start.elapsed().unwrap();
        if elapsed > duration { break; }
    }

    let elapsed = start.elapsed().unwrap();
    let elapsed = (elapsed.as_secs() as f64) +
        (elapsed.subsec_millis() as f64)/1000.0;

    println!("{} {} MB/s", name, (bytes as f64)/(elapsed*1024.0*1024.0));
}


fn main() -> Result<(), Error> {

    let input = proxmox::sys::linux::random_data(1024*1024)?;

    rate_test("zstd", &|| {
        zstd::block::compress(&input, 1).unwrap();
        input.len()
    });

    rate_test("sha256", &|| {
        openssl::sha::sha256(&input);
        input.len()
    });

    let key = proxmox::sys::linux::random_data(32)?;

    let iv = proxmox::sys::linux::random_data(16)?;

    let cipher =  openssl::symm::Cipher::aes_256_gcm();

    rate_test("aes-256-gcm", &|| {
        let mut tag = [0u8;16];
        openssl::symm::encrypt_aead(
            cipher,
            &key,
            Some(&iv),
            b"",
            &input,
            &mut tag).unwrap();
        input.len()
    });

    let cipher =  openssl::symm::Cipher::chacha20_poly1305();

    rate_test("chacha20-poly1305", &|| {
        let mut tag = [0u8;16];
        openssl::symm::encrypt_aead(
            cipher,
            &key,
            Some(&iv),
            b"",
            &input,
            &mut tag).unwrap();
        input.len()
    });

    Ok(())
}
