use anyhow::Error;

//  chacha20-poly1305

fn rate_test(name: &str, bench: &dyn Fn() -> usize) {
    print!("{:<20} ", name);

    let start = std::time::SystemTime::now();
    let duration = std::time::Duration::new(1, 0);

    let mut bytes = 0;

    loop {
        bytes += bench();
        let elapsed = start.elapsed().unwrap();
        if elapsed > duration {
            break;
        }
    }

    let elapsed = start.elapsed().unwrap();
    let elapsed = (elapsed.as_secs() as f64) + (elapsed.subsec_millis() as f64) / 1000.0;

    println!("{:>8.1} MB/s", (bytes as f64) / (elapsed * 1024.0 * 1024.0));
}

fn main() -> Result<(), Error> {
    let input = proxmox_sys::linux::random_data(1024 * 1024)?;

    rate_test("crc32", &|| {
        let mut crchasher = crc32fast::Hasher::new();
        crchasher.update(&input);
        let _checksum = crchasher.finalize();
        input.len()
    });

    rate_test("zstd", &|| {
        zstd::bulk::compress(&input, 1).unwrap();
        input.len()
    });

    rate_test("sha256", &|| {
        openssl::sha::sha256(&input);
        input.len()
    });

    let key = proxmox_sys::linux::random_data(32)?;

    let iv = proxmox_sys::linux::random_data(16)?;

    let cipher = openssl::symm::Cipher::aes_256_gcm();

    rate_test("aes-256-gcm", &|| {
        let mut tag = [0u8; 16];
        openssl::symm::encrypt_aead(cipher, &key, Some(&iv), b"", &input, &mut tag).unwrap();
        input.len()
    });

    let cipher = openssl::symm::Cipher::chacha20_poly1305();

    rate_test("chacha20-poly1305", &|| {
        let mut tag = [0u8; 16];
        openssl::symm::encrypt_aead(cipher, &key, Some(&iv[..12]), b"", &input, &mut tag).unwrap();
        input.len()
    });

    Ok(())
}
