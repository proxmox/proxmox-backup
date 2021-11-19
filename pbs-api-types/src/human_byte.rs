pub struct HumanByte {
    b: usize,
}
impl std::fmt::Display for HumanByte {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.b < 1024 {
            return write!(f, "{} B", self.b);
        }
        let kb: f64 = self.b as f64 / 1024.0;
        if kb < 1024.0 {
            return write!(f, "{:.2} KiB", kb);
        }
        let mb: f64 = kb / 1024.0;
        if mb < 1024.0 {
            return write!(f, "{:.2} MiB", mb);
        }
        let gb: f64 = mb / 1024.0;
        if gb < 1024.0 {
            return write!(f, "{:.2} GiB", gb);
        }
        let tb: f64 = gb / 1024.0;
        if tb < 1024.0 {
            return write!(f, "{:.2} TiB", tb);
        }
        let pb: f64 = tb / 1024.0;
        return write!(f, "{:.2} PiB", pb);
    }
}
impl From<usize> for HumanByte {
    fn from(v: usize) -> Self {
        HumanByte { b: v }
    }
}
impl From<u64> for HumanByte {
    fn from(v: u64) -> Self {
        HumanByte { b: v as usize }
    }
}

#[test]
fn correct_byte_convert() {
    fn convert(b: usize) -> String {
        HumanByte::from(b).to_string()
    }
    assert_eq!(convert(1023), "1023 B");
    assert_eq!(convert(1 << 10), "1.00 KiB");
    assert_eq!(convert(1 << 20), "1.00 MiB");
    assert_eq!(convert((1 << 30) + 103 * (1 << 20)), "1.10 GiB");
    assert_eq!(convert((2 << 50) + 500 * (1 << 40)), "2.49 PiB");
}
