use anyhow::{bail, Error};

/// Size units for byte sizes
#[derive(Debug, Copy, Clone, PartialEq)]
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
            let bits = 63 - (size as u64).leading_zeros();
            match bits {
                50.. => SizeUnit::Pebi,
                40..=49 => SizeUnit::Tebi,
                30..=39 => SizeUnit::Gibi,
                20..=29 => SizeUnit::Mebi,
                10..=19 => SizeUnit::Kibi,
                _ => SizeUnit::Byte,
            }
        } else {
            if size >= 1_000_000_000_000_000.0 {
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
}

/// Returns the string repesentation
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

/// Byte size which can be displayed in a human friendly way
pub struct HumanByte {
    /// The siginficant value, it does not includes any factor of the `unit`
    size: f64,
    /// The scale/unit of the value
    unit: SizeUnit,
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
        HumanByte { size: size / unit.factor(), unit }
    }

    /// Create a new instance with optimal decimal unit computed
    pub fn new_decimal(size: f64) -> Self {
        let unit = SizeUnit::auto_scale(size, false);
        HumanByte { size: size / unit.factor(), unit }
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
