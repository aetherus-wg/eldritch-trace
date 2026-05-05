//! This module defines the BitsRange and BitsMatch types for encoding and matching specific bit fields in event encodings.
//!
use anyhow::{Error, Result, anyhow};
use std::str::FromStr;

/// BitsMatch represents a bitmask and value for matching specific bits in the encoded event. The mask indicates which bits to check, and the value indicates the expected values of those bits.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct BitsMatch {
    /// Bits mask
    pub mask: u32,
    /// Bits value
    pub value: u32,
}

impl BitsMatch {
    /// Checks if the given encoded event matches the BitsMatch criteria by applying the mask and comparing to the value.
    pub fn check(&self, event_encoded: u32) -> bool {
        self.mask & event_encoded == self.value
    }
}

impl From<aetherus_events::filter::BitsMatch> for BitsMatch {
    fn from(other: aetherus_events::filter::BitsMatch) -> Self {
        BitsMatch {
            mask: other.mask,
            value: other.value,
        }
    }
}

impl From<BitsMatch> for aetherus_events::filter::BitsMatch {
    fn from(other: BitsMatch) -> Self {
        aetherus_events::filter::BitsMatch {
            mask: other.mask,
            value: other.value,
        }
    }
}

impl std::fmt::Debug for BitsMatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "BitsMatch {{ mask: 0x{:x}, value: 0x{:x} }}",
            self.mask, self.value
        )
    }
}

/// BitsRange represents a range of bits in the encoded event, specified in "end:start" Verilog style (e.g., "31:28" for bits 31 down to 28).
#[derive(Debug, PartialEq)]
pub struct BitsRange(usize, usize); // (end, start) - "end:start" Verilog style

impl BitsRange {
    /// The size of the range can be calculated as end - start + 1.
    pub fn size(&self) -> usize {
        self.0 - self.1 + 1
    }
}

impl FromStr for BitsRange {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 2 {
            return Err(anyhow!("Invalid bits range format: {}", s));
        }
        let end = parts[0]
            .parse::<usize>()
            .map_err(|e| anyhow!("Invalid start bit: {}", e))?;
        let start = parts[1]
            .parse::<usize>()
            .map_err(|e| anyhow!("Invalid end bit: {}", e))?;
        Ok(BitsRange(end, start))
    }
}

impl BitsMatch {
    /// Parses a string representation of bits (binary, hex, decimal, or don't care) according to the specified BitsRange and returns a BitsMatch
    pub fn parse(input: &str, range: &BitsRange) -> Result<Self> {
        let msb = range.0;
        let lsb = range.1;
        let num_bits = msb - lsb + 1;
        let mask = ((1 << num_bits) - 1) << lsb;
        // input can be:
        // - "0b001011" (binary)
        // - "0x2F" (hex)
        // - "_"/"X" (don't care)
        match input.trim() {
            "X" | "_" => Ok(BitsMatch { mask: 0, value: 0 }),
            s => {
                if let Some(rest) = s.strip_prefix("0b") {
                    // Binary value can also be "0b01xxxxx"
                    let mut mask = 0;
                    let mut value = 0;
                    for c in rest.chars() {
                        match c {
                            'x' => {
                                mask <<= 1;
                                value <<= 1;
                            }
                            '0' | '1' => {
                                mask = (mask << 1) | 1;
                                value = (value << 1) | if c == '1' { 1 } else { 0 };
                            }
                            _ => {
                                return Err(anyhow!(
                                    "Invalid character '{}' in binary literal '{}'",
                                    c,
                                    s
                                ));
                            }
                        }
                    }
                    Ok(BitsMatch {
                        mask: mask << lsb,
                        value: value << lsb,
                    })
                } else if let Some(rest) = s.strip_prefix("0x") {
                    Ok(BitsMatch {
                        mask,
                        value: u32::from_str_radix(rest, 16)
                            .map_err(|e| anyhow!("Invalid hex value '{}': {}", rest, e))?
                            << lsb,
                    })
                } else {
                    Ok(BitsMatch {
                        mask,
                        value: s
                            .parse::<u32>()
                            .map_err(|e| anyhow!("Invalid integer value '{}': {}", s, e))?
                            << lsb,
                    })
                }
            }
        }
    }

    /// Combines two BitsMatch instances by merging their masks and values.
    /// WARN: If there are overlapping bits in the masks, they must have the same value for those bits to be combined successfully.
    pub fn combine(&self, other: &BitsMatch) -> BitsMatch {
        assert!(
            self.mask & other.mask == 0
                || (self.mask & other.mask & self.value) == (self.mask & other.mask & other.value),
            "Cannot combine BitsMatch with ambiguous values in overlapping masks"
        );
        BitsMatch {
            mask: self.mask | other.mask,
            value: (self.mask & self.value) | (other.mask & other.value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_range_size() {
        assert_eq!(BitsRange(31, 28).size(), 4);
        assert_eq!(BitsRange(15, 0).size(), 16);
        assert_eq!(BitsRange(23, 16).size(), 8);
    }

    #[test]
    fn bits_range_parse_valid() {
        assert_eq!(BitsRange::from_str("31:28").unwrap(), BitsRange(31, 28));
        assert_eq!(BitsRange::from_str("15:0").unwrap(), BitsRange(15, 0));
        assert_eq!(BitsRange::from_str("7:0").unwrap(), BitsRange(7, 0));
    }

    #[test]
    fn bits_range_parse_invalid() {
        assert!(BitsRange::from_str("invalid").is_err());
        assert!(BitsRange::from_str("31-28").is_err());
        assert!(BitsRange::from_str("31:").is_err());
        assert!(BitsRange::from_str(":28").is_err());
    }

    #[test]
    fn bits_match_check() {
        let bm = BitsMatch {
            mask: 0xF0000000,
            value: 0x10000000,
        };
        assert!(bm.check(0x12345678));
        assert!(!bm.check(0x02345678));
        assert!(!bm.check(0x20000000));
    }

    #[test]
    fn bits_match_dont_care() {
        let bm = BitsMatch { mask: 0, value: 0 };
        assert!(bm.check(0xFFFFFFFF));
        assert!(bm.check(0x00000000));
    }

    #[test]
    fn bits_match_parse_binary() {
        let range = BitsRange(27, 24);
        let bm = BitsMatch::parse("0b0011", &range).unwrap();
        assert_eq!(bm.mask, 0x0F000000);
        assert_eq!(bm.value, 0x03000000);
    }

    #[test]
    fn bits_match_parse_binary_with_x() {
        let range = BitsRange(7, 0);
        let bm = BitsMatch::parse("0bxxxxxxx1", &range).unwrap();
        assert_eq!(bm.mask, 0x00000001);
        assert!(bm.check(0x00000001));
        assert!(bm.check(0x00000003));
        assert!(!bm.check(0x00000002));
    }

    #[test]
    fn bits_match_parse_hex() {
        let range = BitsRange(15, 0);
        let bm = BitsMatch::parse("0x002F", &range).unwrap();
        assert_eq!(bm.mask, 0xFFFF);
        assert_eq!(bm.value, 0x002F);
    }

    #[test]
    fn bits_match_parse_decimal() {
        let range = BitsRange(23, 16);
        let bm = BitsMatch::parse("42", &range).unwrap();
        assert_eq!(bm.mask, 0x00FF0000);
        assert_eq!(bm.value, 0x002A0000);
    }

    #[test]
    fn bits_match_parse_dont_care() {
        let range = BitsRange(31, 28);
        let bm = BitsMatch::parse("_", &range).unwrap();
        assert_eq!(bm, BitsMatch { mask: 0, value: 0 });

        let bm2 = BitsMatch::parse("X", &range).unwrap();
        assert_eq!(bm2, BitsMatch { mask: 0, value: 0 });
    }

    #[test]
    fn bits_match_combine() {
        let bm1 = BitsMatch {
            mask: 0xF0000000,
            value: 0x10000000,
        };
        let bm2 = BitsMatch {
            mask: 0x0F000000,
            value: 0x03000000,
        };
        let combined = bm1.combine(&bm2);
        assert_eq!(combined.mask, 0xFF000000);
        assert_eq!(combined.value, 0x13000000);
    }

    #[test]
    fn bits_match_combine_overlapping() {
        let bm1 = BitsMatch {
            mask: 0xFFFF0000,
            value: 0x12340000,
        };
        let bm2 = BitsMatch {
            mask: 0x0000FFFF,
            value: 0x00005678,
        };
        let combined = bm1.combine(&bm2);
        assert_eq!(combined.mask, 0xFFFFFFFF);
        assert_eq!(combined.value, 0x12345678);
    }

    #[test]
    fn bits_match_combine_ambiguous_fails() {
        let bm1 = BitsMatch {
            mask: 0xFF000000,
            value: 0x10000000,
        };
        let bm2 = BitsMatch {
            mask: 0xFF000000,
            value: 0x20000000,
        };
        let result = std::panic::catch_unwind(|| bm1.combine(&bm2));
        assert!(result.is_err());
    }
}
