//! Identity: UUIDs, derived UUIDv5 ids, human labels and IFC GlobalIds
//! (spec 5.5, D-12).
//!
//! Derived ids are UUIDv5 under [`PROJECT_NAMESPACE`]. The hashed name for
//! `(parent, operator, slot)` is the parent's 16 bytes, then the operator,
//! then the slot, each string written as a 4-byte big-endian byte length
//! followed by its UTF-8 bytes. The length prefixes make the encoding
//! injective, so distinct triples never share a name.

use std::collections::BTreeMap;
use std::fmt;

/// The namespace for every derived id: UUIDv5 of the URL
/// `https://github.com/FusionAtom360/AIArchViz` under the RFC 4122 URL
/// namespace, written out so it can never drift.
pub const PROJECT_NAMESPACE: Uuid = Uuid(uuid::Uuid::from_bytes([
    0xc8, 0xf4, 0x47, 0xff, 0xf3, 0x98, 0x5d, 0x52, 0xaf, 0x72, 0xbc, 0x46, 0x2c, 0xec, 0x31, 0x6a,
]));

/// The IFC base-64 alphabet (IFC 4.3.2.0, 8.21.2.2.1).
const IFC_BASE64: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$";

/// An entity identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uuid(uuid::Uuid);

impl Uuid {
    /// The UUID with these 16 bytes, big-endian as in RFC 4122.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Uuid(uuid::Uuid::from_bytes(bytes))
    }

    /// The 16 bytes, big-endian.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        self.0.as_bytes()
    }

    /// The id of the child that `operator` derives from `parent` in `slot`.
    ///
    /// A string longer than `u32::MAX` bytes has its length prefix saturated;
    /// no real operator or slot name comes near that.
    pub fn derive(parent: Uuid, operator: &str, slot: &str) -> Self {
        let mut name = Vec::with_capacity(16 + 8 + operator.len() + slot.len());
        name.extend_from_slice(parent.as_bytes());
        for part in [operator, slot] {
            let len = u32::try_from(part.len()).unwrap_or(u32::MAX);
            name.extend_from_slice(&len.to_be_bytes());
            name.extend_from_slice(part.as_bytes());
        }
        Uuid(uuid::Uuid::new_v5(&PROJECT_NAMESPACE.0, &name))
    }

    /// The 22-character IFC GlobalId (IfcGloballyUniqueId).
    ///
    /// The 128 bits are written big-endian in base 64 with the IFC alphabet:
    /// the first character carries the top 2 bits (so it is 0 to 3), and
    /// each of the other 21 carries 6 bits.
    pub fn global_id(&self) -> String {
        let v = u128::from_be_bytes(*self.as_bytes());
        let digit = |shift: u32| IFC_BASE64[((v >> shift) & 63) as usize] as char;
        let mut out = String::with_capacity(22);
        out.push(digit(126));
        for i in (0..21).rev() {
            out.push(digit(6 * i));
        }
        out
    }
}

impl fmt::Display for Uuid {
    /// Lowercase hyphenated form, e.g. `f70dd363-bfe3-495d-84a0-2c02dcb7d4d2`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0.hyphenated(), f)
    }
}

/// A human label such as `W-007` or `EQ-012`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Label {
    prefix: String,
    number: u32,
}

impl Label {
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    pub const fn number(&self) -> u32 {
        self.number
    }
}

impl fmt::Display for Label {
    /// Prefix, hyphen, number zero-padded to at least three digits.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{:03}", self.prefix, self.number)
    }
}

/// Why a label could not be allocated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelError {
    /// A prefix must be one or more ASCII uppercase letters.
    InvalidPrefix(String),
    /// Every number for this prefix has been issued.
    Exhausted(String),
}

impl fmt::Display for LabelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LabelError::InvalidPrefix(p) => {
                write!(f, "label prefix {p:?} must be ASCII uppercase letters")
            }
            LabelError::Exhausted(p) => write!(f, "no labels left for prefix {p}"),
        }
    }
}

impl std::error::Error for LabelError {}

/// Issues labels per prefix. Numbers only ever increase, so a label is never
/// reissued, even after the entity that held it is deleted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelAllocator {
    /// The last number issued for each prefix.
    last: BTreeMap<String, u32>,
}

impl LabelAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    /// The next unused label for `prefix`.
    pub fn allocate(&mut self, prefix: &str) -> Result<Label, LabelError> {
        if prefix.is_empty() || !prefix.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(LabelError::InvalidPrefix(prefix.to_string()));
        }
        let number = self
            .last
            .get(prefix)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| LabelError::Exhausted(prefix.to_string()))?;
        self.last.insert(prefix.to_string(), number);
        Ok(Label {
            prefix: prefix.to_string(),
            number,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeSet;

    fn id(s: &str) -> Uuid {
        Uuid(uuid::Uuid::parse_str(s).unwrap_or_default())
    }

    #[test]
    fn namespace_is_the_documented_derivation() {
        let derived = uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            b"https://github.com/FusionAtom360/AIArchViz",
        );
        assert_eq!(PROJECT_NAMESPACE.0, derived);
        assert_eq!(
            PROJECT_NAMESPACE.to_string(),
            "c8f447ff-f398-5d52-af72-bc462cec316a"
        );
    }

    /// (operator, slot, expected id) for parent f70dd363-...-2c02dcb7d4d2.
    /// Expected values were computed independently with Python's
    /// `uuid.uuid5` over the documented name encoding. Fixed literals make
    /// the test fail if derivation ever differs between runs, processes,
    /// machines or versions.
    const DERIVED: &[(&str, &str, &str)] = &[
        ("baseboard", "left", "8c4c8f0a-f599-5677-90ea-db2dff3b4c7a"),
        ("baseboard", "right", "14f5ac7c-c1c8-5214-b119-f0f531ff75c5"),
        (
            "wall_finish",
            "left",
            "4b6ec929-70af-5e15-bd3a-48fe6988568d",
        ),
        ("", "", "68f83f76-1959-552c-92b5-738fcbd9727a"),
        ("ab", "c", "95282116-fdb8-5c0b-aaf8-3c76cf3ad5f8"),
        ("a", "bc", "adc9e550-c5db-52b7-9650-f44837fe0d45"),
    ];

    #[test]
    fn derivation_matches_fixed_values() {
        let parent = id("f70dd363-bfe3-495d-84a0-2c02dcb7d4d2");
        for &(operator, slot, expected) in DERIVED {
            let child = Uuid::derive(parent, operator, slot);
            assert_eq!(child.to_string(), expected, "{operator:?} {slot:?}");
            assert_eq!(child.0.get_version_num(), 5);
        }
        let nil_parent = Uuid::from_bytes([0; 16]);
        assert_eq!(
            Uuid::derive(nil_parent, "baseboard", "left").to_string(),
            "c749370b-2b39-5bae-8524-02e5f8941657"
        );
    }

    #[test]
    fn derivation_separates_operator_and_slot() {
        // Without length prefixes ("ab","c") and ("a","bc") would collide.
        let parent = id("f70dd363-bfe3-495d-84a0-2c02dcb7d4d2");
        assert_ne!(
            Uuid::derive(parent, "ab", "c"),
            Uuid::derive(parent, "a", "bc")
        );
    }

    /// GlobalId test vectors.
    ///
    /// 1. buildingSMART, IFC 4.3.2.0, section 8.21.2.2.1 IfcGloballyUniqueId,
    ///    https://standards.buildingsmart.org/IFC/RELEASE/IFC4_3/HTML/lexical/IfcGloballyUniqueId.htm :
    ///    "the generated base16 number f70dd363-bfe3-495d-84a0-2c02dcb7d4d2
    ///    will compress to 3t3TDZl_D9NOIWB0BSjzJI".
    /// 2. IfcOpenShell v0.8.0, src/ifcopenshell-python/test/test_guid.py,
    ///    the padding test, which builds `"ff" + "0" * 30` and expects
    ///    `CHARS[255 // 64] + CHARS[255 % 64] + "0" * 20` = `"3$" + "0" * 20`
    ///    ("IFC's hex to base64 expansion should isolate first two digits").
    /// 3. The extremes implied by the same section's rule that the first
    ///    character "must be either a 0, 1, 2, or 3": all-zero and all-one bits.
    const GLOBAL_IDS: &[(&str, &str)] = &[
        (
            "f70dd363-bfe3-495d-84a0-2c02dcb7d4d2",
            "3t3TDZl_D9NOIWB0BSjzJI",
        ),
        (
            "ff000000-0000-0000-0000-000000000000",
            "3$00000000000000000000",
        ),
        (
            "00000000-0000-0000-0000-000000000000",
            "0000000000000000000000",
        ),
        (
            "ffffffff-ffff-ffff-ffff-ffffffffffff",
            "3$$$$$$$$$$$$$$$$$$$$$",
        ),
    ];

    #[test]
    fn global_id_matches_published_vectors() {
        for &(uuid, expected) in GLOBAL_IDS {
            assert_eq!(id(uuid).global_id(), expected, "{uuid}");
        }
    }

    const IFC_ALPHABET: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$";

    /// Decodes a GlobalId back to bytes, independently of the encoder.
    fn decode(g: &str) -> Option<[u8; 16]> {
        let mut v: u128 = 0;
        for c in g.chars() {
            let d = IFC_ALPHABET.find(c)?;
            v = v.checked_mul(64)?.checked_add(u128::try_from(d).ok()?)?;
        }
        Some(v.to_be_bytes())
    }

    #[test]
    fn label_format() {
        let mut a = LabelAllocator::new();
        let labels: Vec<String> = (0..3)
            .map(|_| a.allocate("W").map(|l| l.to_string()))
            .collect::<Result<_, _>>()
            .unwrap_or_default();
        assert_eq!(labels, ["W-001", "W-002", "W-003"]);
        assert_eq!(a.allocate("EQ").map(|l| l.to_string()), Ok("EQ-001".into()));
        let wide = Label {
            prefix: "W".into(),
            number: 1234,
        };
        assert_eq!(wide.to_string(), "W-1234");
    }

    #[test]
    fn prefixes_count_independently() {
        let mut a = LabelAllocator::new();
        for _ in 0..7 {
            let _ = a.allocate("W");
        }
        let eq = a.allocate("EQ");
        assert_eq!(eq.as_ref().map(Label::number), Ok(1));
        assert_eq!(eq.as_ref().map(Label::prefix), Ok("EQ"));
        assert_eq!(a.allocate("W").map(|l| l.number()), Ok(8));
    }

    #[test]
    fn invalid_prefixes_are_rejected() {
        let mut a = LabelAllocator::new();
        for bad in ["", "w", "W1", "W-", "É", " W"] {
            assert_eq!(
                a.allocate(bad),
                Err(LabelError::InvalidPrefix(bad.to_string())),
                "{bad:?}"
            );
        }
        // A rejected prefix consumes nothing.
        assert_eq!(a.allocate("W").map(|l| l.number()), Ok(1));
    }

    #[test]
    fn deleted_labels_are_never_reissued() {
        let mut a = LabelAllocator::new();
        let mut live: BTreeSet<Label> = BTreeSet::new();
        for _ in 0..3 {
            live.extend(a.allocate("W"));
        }
        // Delete the newest and an older one; the allocator is not told,
        // and must not hand either out again.
        let w3 = Label {
            prefix: "W".into(),
            number: 3,
        };
        let w1 = Label {
            prefix: "W".into(),
            number: 1,
        };
        assert!(live.remove(&w3) && live.remove(&w1));
        let next = a.allocate("W");
        assert_eq!(
            next.as_ref().map(ToString::to_string),
            Ok("W-004".to_string())
        );
    }

    #[test]
    fn exhausted_prefix_is_an_error_never_a_wrap() {
        let mut a = LabelAllocator::new();
        a.last.insert("W".into(), u32::MAX - 1);
        assert_eq!(a.allocate("W").map(|l| l.number()), Ok(u32::MAX));
        assert_eq!(a.allocate("W"), Err(LabelError::Exhausted("W".into())));
        assert_eq!(a.allocate("W"), Err(LabelError::Exhausted("W".into())));
    }

    proptest! {
        /// Any GlobalId is 22 IFC characters, starts with 0 to 3, and decodes
        /// back to the same 128 bits.
        #[test]
        fn global_id_is_22_ifc_chars_and_round_trips(bytes in any::<[u8; 16]>()) {
            let g = Uuid::from_bytes(bytes).global_id();
            prop_assert_eq!(g.chars().count(), 22);
            prop_assert!(g.chars().all(|c| IFC_ALPHABET.contains(c)));
            prop_assert!(g.starts_with(['0', '1', '2', '3']));
            prop_assert_eq!(decode(&g), Some(bytes));
        }

        /// Across any mix of allocations and deletions, every issued label is new.
        #[test]
        fn labels_are_unique_under_any_deletions(
            ops in proptest::collection::vec((0usize..3, any::<bool>()), 0..200)
        ) {
            const PREFIXES: [&str; 3] = ["W", "EQ", "L"];
            let mut a = LabelAllocator::new();
            let mut issued = BTreeSet::new();
            let mut live: Vec<Label> = Vec::new();
            for (p, delete) in ops {
                if delete && !live.is_empty() {
                    live.remove(p % live.len());
                } else {
                    let label = a.allocate(PREFIXES[p]);
                    prop_assert!(label.is_ok());
                    let label = label.map_err(|e| TestCaseError::fail(e.to_string()))?;
                    prop_assert!(issued.insert(label.clone()), "reissued {}", label);
                    live.push(label);
                }
            }
        }

        /// Derivation is a pure function of its inputs.
        #[test]
        fn derivation_is_deterministic(
            parent in any::<[u8; 16]>(),
            operator in ".{0,12}",
            slot in ".{0,12}",
        ) {
            let p = Uuid::from_bytes(parent);
            prop_assert_eq!(
                Uuid::derive(p, &operator, &slot),
                Uuid::derive(p, &operator, &slot)
            );
        }
    }
}
