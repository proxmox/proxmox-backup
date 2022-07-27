use anyhow::{bail, Error};

use proxmox_schema::{ApiStringFormat, ApiType, Schema, StringSchema, UpdaterType};

/// Size units for byte sizes
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SizeUnit {
    Byte,
    // SI (base 10)
    KByte,
    MByte,
    GByte,
    TByte,
    PByte,
    // IEC (base 2)
    Kibi,
    Mebi,
    Gibi,
    Tebi,
    Pebi,
}

impl SizeUnit {
    /// Returns the scaling factor
    pub fn factor(&self) -> f64 {
        match self {
            SizeUnit::Byte => 1.0,
            // SI (base 10)
            SizeUnit::KByte => 1_000.0,
            SizeUnit::MByte => 1_000_000.0,
            SizeUnit::GByte => 1_000_000_000.0,
            SizeUnit::TByte => 1_000_000_000_000.0,
            SizeUnit::PByte => 1_000_000_000_000_000.0,
            // IEC (base 2)
            SizeUnit::Kibi => 1024.0,
            SizeUnit::Mebi => 1024.0 * 1024.0,
            SizeUnit::Gibi => 1024.0 * 1024.0 * 1024.0,
            SizeUnit::Tebi => 1024.0 * 1024.0 * 1024.0 * 1024.0,
            SizeUnit::Pebi => 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0,
        }
    }

    /// gets the biggest possible unit still having a value greater zero before the decimal point
    /// 'binary' specifies if IEC (base 2) units should be used or SI (base 10) ones
    pub fn auto_scale(size: f64, binary: bool) -> SizeUnit {
        if binary {
            let bits = 64 - (size as u64).leading_zeros();
            match bits {
                51.. => SizeUnit::Pebi,
                41..=50 => SizeUnit::Tebi,
                31..=40 => SizeUnit::Gibi,
                21..=30 => SizeUnit::Mebi,
                11..=20 => SizeUnit::Kibi,
                _ => SizeUnit::Byte,
            }
        } else if size >= 1_000_000_000_000_000.0 {
            SizeUnit::PByte
        } else if size >= 1_000_000_000_000.0 {
            SizeUnit::TByte
        } else if size >= 1_000_000_000.0 {
            SizeUnit::GByte
        } else if size >= 1_000_000.0 {
            SizeUnit::MByte
        } else if size >= 1_000.0 {
            SizeUnit::KByte
        } else {
            SizeUnit::Byte
        }
    }
}

/// Returns the string representation
impl std::fmt::Display for SizeUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SizeUnit::Byte => write!(f, "B"),
            // SI (base 10)
            SizeUnit::KByte => write!(f, "KB"),
            SizeUnit::MByte => write!(f, "MB"),
            SizeUnit::GByte => write!(f, "GB"),
            SizeUnit::TByte => write!(f, "TB"),
            SizeUnit::PByte => write!(f, "PB"),
            // IEC (base 2)
            SizeUnit::Kibi => write!(f, "KiB"),
            SizeUnit::Mebi => write!(f, "MiB"),
            SizeUnit::Gibi => write!(f, "GiB"),
            SizeUnit::Tebi => write!(f, "TiB"),
            SizeUnit::Pebi => write!(f, "PiB"),
        }
    }
}

/// Strips a trailing SizeUnit inclusive trailing whitespace
/// Supports both IEC and SI based scales, the B/b byte symbol is optional.
fn strip_unit(v: &str) -> (&str, SizeUnit) {
    let v = v.strip_suffix(&['b', 'B'][..]).unwrap_or(v); // byte is implied anyway

    let (v, binary) = match v.strip_suffix('i') {
        Some(n) => (n, true),
        None => (v, false),
    };

    let mut unit = SizeUnit::Byte;
    #[rustfmt::skip]
    let value = v.strip_suffix(|c: char| match c {
        'k' | 'K' if !binary => { unit = SizeUnit::KByte; true }
        'm' | 'M' if !binary => { unit = SizeUnit::MByte; true }
        'g' | 'G' if !binary => { unit = SizeUnit::GByte; true }
        't' | 'T' if !binary => { unit = SizeUnit::TByte; true }
        'p' | 'P' if !binary => { unit = SizeUnit::PByte; true }
        // binary (IEC recommended) variants
        'k' | 'K' if binary => { unit = SizeUnit::Kibi; true }
        'm' | 'M' if binary => { unit = SizeUnit::Mebi; true }
        'g' | 'G' if binary => { unit = SizeUnit::Gibi; true }
        't' | 'T' if binary => { unit = SizeUnit::Tebi; true }
        'p' | 'P' if binary => { unit = SizeUnit::Pebi; true }
        _ => false
    }).unwrap_or(v).trim_end();

    (value, unit)
}

/// Byte size which can be displayed in a human friendly way
#[derive(Debug, Copy, Clone, UpdaterType)]
pub struct HumanByte {
    /// The siginficant value, it does not includes any factor of the `unit`
    size: f64,
    /// The scale/unit of the value
    unit: SizeUnit,
}

fn verify_human_byte(s: &str) -> Result<(), Error> {
    match s.parse::<HumanByte>() {
        Ok(_) => Ok(()),
        Err(err) => bail!("byte-size parse error for '{}': {}", s, err),
    }
}
impl ApiType for HumanByte {
    const API_SCHEMA: Schema = StringSchema::new(
        "Byte size with optional unit (B, KB (base 10), MB, GB, ..., KiB (base 2), MiB, Gib, ...).",
    )
    .format(&ApiStringFormat::VerifyFn(verify_human_byte))
    .min_length(1)
    .max_length(64)
    .schema();
}

impl HumanByte {
    /// Create instance with size and unit (size must be positive)
    pub fn with_unit(size: f64, unit: SizeUnit) -> Result<Self, Error> {
        if size < 0.0 {
            bail!("byte size may not be negative");
        }
        Ok(HumanByte { size, unit })
    }

    /// Create a new instance with optimal binary unit computed
    pub fn new_binary(size: f64) -> Self {
        let unit = SizeUnit::auto_scale(size, true);
        HumanByte {
            size: size / unit.factor(),
            unit,
        }
    }

    /// Create a new instance with optimal decimal unit computed
    pub fn new_decimal(size: f64) -> Self {
        let unit = SizeUnit::auto_scale(size, false);
        HumanByte {
            size: size / unit.factor(),
            unit,
        }
    }

    /// Returns the size as u64 number of bytes
    pub fn as_u64(&self) -> u64 {
        self.as_f64() as u64
    }

    /// Returns the size as f64 number of bytes
    pub fn as_f64(&self) -> f64 {
        self.size * self.unit.factor()
    }

    /// Returns a copy with optimal binary unit computed
    pub fn auto_scale_binary(self) -> Self {
        HumanByte::new_binary(self.as_f64())
    }

    /// Returns a copy with optimal decimal unit computed
    pub fn auto_scale_decimal(self) -> Self {
        HumanByte::new_decimal(self.as_f64())
    }
}

impl From<u64> for HumanByte {
    fn from(v: u64) -> Self {
        HumanByte::new_binary(v as f64)
    }
}
impl From<usize> for HumanByte {
    fn from(v: usize) -> Self {
        HumanByte::new_binary(v as f64)
    }
}

impl std::fmt::Display for HumanByte {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let precision = f.precision().unwrap_or(3) as f64;
        let precision_factor = 1.0 * 10.0_f64.powf(precision);
        // this could cause loss of information, rust has sadly no shortest-max-X flt2dec fmt yet
        let size = ((self.size * precision_factor).round()) / precision_factor;
        write!(f, "{} {}", size, self.unit)
    }
}

impl std::str::FromStr for HumanByte {
    type Err = Error;

    fn from_str(v: &str) -> Result<Self, Error> {
        let (v, unit) = strip_unit(v);
        HumanByte::with_unit(v.parse()?, unit)
    }
}

proxmox_serde::forward_deserialize_to_from_str!(HumanByte);
proxmox_serde::forward_serialize_to_display!(HumanByte);

#[test]
fn test_human_byte_parser() -> Result<(), Error> {
    assert!("-10".parse::<HumanByte>().is_err()); // negative size

    fn do_test(v: &str, size: f64, unit: SizeUnit, as_str: &str) -> Result<(), Error> {
        let h: HumanByte = v.parse()?;

        if h.size != size {
            bail!("got unexpected size for '{}' ({} != {})", v, h.size, size);
        }
        if h.unit != unit {
            bail!(
                "got unexpected unit for '{}' ({:?} != {:?})",
                v,
                h.unit,
                unit
            );
        }

        let new = h.to_string();
        if &new != as_str {
            bail!("to_string failed for '{}' ({:?} != {:?})", v, new, as_str);
        }
        Ok(())
    }
    fn test(v: &str, size: f64, unit: SizeUnit, as_str: &str) -> bool {
        match do_test(v, size, unit, as_str) {
            Ok(_) => true,
            Err(err) => {
                eprintln!("{}", err); // makes debugging easier
                false
            }
        }
    }

    assert!(test("14", 14.0, SizeUnit::Byte, "14 B"));
    assert!(test("14.4", 14.4, SizeUnit::Byte, "14.4 B"));
    assert!(test("14.45", 14.45, SizeUnit::Byte, "14.45 B"));
    assert!(test("14.456", 14.456, SizeUnit::Byte, "14.456 B"));
    assert!(test("14.4567", 14.4567, SizeUnit::Byte, "14.457 B"));

    let h: HumanByte = "1.2345678".parse()?;
    assert_eq!(&format!("{:.0}", h), "1 B");
    assert_eq!(&format!("{:.0}", h.as_f64()), "1"); // use as_f64 to get raw bytes without unit
    assert_eq!(&format!("{:.1}", h), "1.2 B");
    assert_eq!(&format!("{:.2}", h), "1.23 B");
    assert_eq!(&format!("{:.3}", h), "1.235 B");
    assert_eq!(&format!("{:.4}", h), "1.2346 B");
    assert_eq!(&format!("{:.5}", h), "1.23457 B");
    assert_eq!(&format!("{:.6}", h), "1.234568 B");
    assert_eq!(&format!("{:.7}", h), "1.2345678 B");
    assert_eq!(&format!("{:.8}", h), "1.2345678 B");

    assert!(test(
        "987654321",
        987654321.0,
        SizeUnit::Byte,
        "987654321 B"
    ));

    assert!(test("1300b", 1300.0, SizeUnit::Byte, "1300 B"));
    assert!(test("1300B", 1300.0, SizeUnit::Byte, "1300 B"));
    assert!(test("1300 B", 1300.0, SizeUnit::Byte, "1300 B"));
    assert!(test("1300 b", 1300.0, SizeUnit::Byte, "1300 B"));

    assert!(test("1.5KB", 1.5, SizeUnit::KByte, "1.5 KB"));
    assert!(test("1.5kb", 1.5, SizeUnit::KByte, "1.5 KB"));
    assert!(test("1.654321MB", 1.654_321, SizeUnit::MByte, "1.654 MB"));

    assert!(test("2.0GB", 2.0, SizeUnit::GByte, "2 GB"));

    assert!(test("1.4TB", 1.4, SizeUnit::TByte, "1.4 TB"));
    assert!(test("1.4tb", 1.4, SizeUnit::TByte, "1.4 TB"));

    assert!(test("2KiB", 2.0, SizeUnit::Kibi, "2 KiB"));
    assert!(test("2Ki", 2.0, SizeUnit::Kibi, "2 KiB"));
    assert!(test("2kib", 2.0, SizeUnit::Kibi, "2 KiB"));

    assert!(test("2.3454MiB", 2.3454, SizeUnit::Mebi, "2.345 MiB"));
    assert!(test("2.3456MiB", 2.3456, SizeUnit::Mebi, "2.346 MiB"));

    assert!(test("4gib", 4.0, SizeUnit::Gibi, "4 GiB"));

    Ok(())
}

#[test]
fn test_human_byte_auto_unit_decimal() {
    fn convert(b: u64) -> String {
        HumanByte::new_decimal(b as f64).to_string()
    }
    assert_eq!(convert(987), "987 B");
    assert_eq!(convert(1022), "1.022 KB");
    assert_eq!(convert(9_000), "9 KB");
    assert_eq!(convert(1_000), "1 KB");
    assert_eq!(convert(1_000_000), "1 MB");
    assert_eq!(convert(1_000_000_000), "1 GB");
    assert_eq!(convert(1_000_000_000_000), "1 TB");
    assert_eq!(convert(1_000_000_000_000_000), "1 PB");

    assert_eq!(convert((1 << 30) + 103 * (1 << 20)), "1.182 GB");
    assert_eq!(convert((1 << 30) + 128 * (1 << 20)), "1.208 GB");
    assert_eq!(convert((2 << 50) + 500 * (1 << 40)), "2.802 PB");
}

#[test]
fn test_human_byte_auto_unit_binary() {
    fn convert(b: u64) -> String {
        HumanByte::from(b).to_string()
    }
    assert_eq!(convert(0), "0 B");
    assert_eq!(convert(987), "987 B");
    assert_eq!(convert(1022), "1022 B");
    assert_eq!(convert(9_000), "8.789 KiB");
    assert_eq!(convert(10_000_000), "9.537 MiB");
    assert_eq!(convert(10_000_000_000), "9.313 GiB");
    assert_eq!(convert(10_000_000_000_000), "9.095 TiB");

    assert_eq!(convert(1 << 10), "1 KiB");
    assert_eq!(convert((1 << 10) * 10), "10 KiB");
    assert_eq!(convert(1 << 20), "1 MiB");
    assert_eq!(convert(1 << 30), "1 GiB");
    assert_eq!(convert(1 << 40), "1 TiB");
    assert_eq!(convert(1 << 50), "1 PiB");

    assert_eq!(convert((1 << 30) + 103 * (1 << 20)), "1.101 GiB");
    assert_eq!(convert((1 << 30) + 128 * (1 << 20)), "1.125 GiB");
    assert_eq!(convert((1 << 40) + 128 * (1 << 30)), "1.125 TiB");
    assert_eq!(convert((2 << 50) + 512 * (1 << 40)), "2.5 PiB");
}
