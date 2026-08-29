//! Production braid representation, mirror-orbit normalizer and action transport.
//!
//! The geometry is a cylinder: the word/time direction is cyclic, while the
//! strand direction has a marked first boundary. The v1 graph key additionally
//! quotients by global mirror reflection with an explicit parity witness.

use crate::{RepKey, Result};

pub const REPRESENTATION_CODEC: &str = "unknotdb-braid-cylinder-le-v0";
pub const PACKED_REPRESENTATION_CODEC: &str = "unknotdb-braid-cylinder-packed-v1";
pub const NORMALIZER_VERSION: &str = "mirror-writhe-word-necklace-v1";
pub const KEY_SPEC: &str = "sha256(mirror-orbit-normalized-unknotdb-braid-cylinder-v1)";
pub const EXACT_CHECKPOINT_KEY_SPEC: &str =
    "sha256(UNKNOTDB_EXACT_BRAID_CHECKPOINT_V0\\0 || representation-v0-encoding)";
pub const ACTION_CODEC: &str = "unknotdb-semantic-action-u63-v0";
pub const PROGRAM_CODEC: &str = "unknotdb-semantic-program-le-v0";
pub const CHECKPOINTED_PROGRAM_CODEC: &str = "unknotdb-semantic-checkpoint-program-le-v1";
pub const ANCHORED_PROGRAM_CODEC: &str = "unknotdb-relative-anchor-program-v1";
pub const VALIDATOR_VERSION: &str = "unknotdb-runtime-braid-validator-v1";

const MAGIC: [u8; 4] = *b"UKB0";
const HEADER_LEN: usize = 12;
const PACKED_MAGIC: u8 = 0xb1;
const CYCLIC_BAND_FLAG: u8 = 1;
const NIBBLE_WORD_FLAG: u8 = 2;
const BYTE_WORD_FLAG: u8 = 4;
const PROGRAM_MAGIC: [u8; 4] = *b"UKP0";
const PROGRAM_HEADER_LEN: usize = 12;
const NORMALIZE_ORIGIN_TAG: u64 = 1 << 63;
const ORBIT_OPCODE_SHIFT: u32 = 61;
const ORBIT_OPCODE_MASK: u64 = 0b11 << ORBIT_OPCODE_SHIFT;
const NORMALIZE_ORIGIN_OPCODE: u64 = 0;
const ROTATE_ORIGIN_OPCODE: u64 = 1;
const MIRROR_ORBIT_OPCODE: u64 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BraidRepresentation {
    pub strands: u16,
    pub cyclic_band_generators: bool,
    pub word: Vec<i16>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NormalizationWitness {
    /// Whether the representation was first reflected by `letter -> -letter`.
    pub mirrored: bool,
    /// `normalized[k] = input[(k + rotate_word_left) mod length]`.
    /// When `mirrored`, `input` in this formula means the reflected word.
    pub rotate_word_left: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormalizedRepresentation {
    pub representation: BraidRepresentation,
    pub witness: NormalizationWitness,
    pub encoded: Vec<u8>,
    pub key: RepKey,
}

impl BraidRepresentation {
    /// Domain-separated identity for an exact, possibly unnormalized proof
    /// checkpoint. Unlike [`Self::normalize`], this does not quotient cyclic
    /// origin or mirror reflection.
    pub fn exact_checkpoint_key(&self) -> Result<RepKey> {
        self.validate()?;
        let encoded = self.encode()?;
        let mut material = Vec::with_capacity(36 + encoded.len());
        material.extend_from_slice(b"UNKNOTDB_EXACT_BRAID_CHECKPOINT_V0\0");
        material.extend_from_slice(&encoded);
        sha256_key(&material)
    }

    pub fn validate(&self) -> Result<()> {
        if self.strands == 0 {
            return Err("a braid must have at least one strand".into());
        }
        if self.strands > i16::MAX as u16 {
            return Err(format!(
                "{} strands exceed the v0 signed-letter capacity",
                self.strands
            )
            .into());
        }
        let largest = if self.cyclic_band_generators {
            self.strands
        } else {
            self.strands.saturating_sub(1)
        };
        for (position, &letter) in self.word.iter().enumerate() {
            if letter == 0 {
                return Err(
                    format!("zero padding at word position {} is not encoded", position).into(),
                );
            }
            let generator = letter.unsigned_abs();
            if generator == 0 || generator > largest {
                return Err(format!(
                    "generator {} at position {} is invalid for {} strands",
                    letter, position, self.strands
                )
                .into());
            }
            if self.cyclic_band_generators && generator == self.strands && self.strands < 3 {
                return Err(
                    "the cyclic seam generator is distinct only from three strands on".into(),
                );
            }
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let word_len: u32 = self
            .word
            .len()
            .try_into()
            .map_err(|_| "braid word is too long for the v0 codec")?;
        let mut out = Vec::with_capacity(HEADER_LEN + self.word.len() * 2);
        out.extend_from_slice(&MAGIC);
        out.push(0); // codec version
        out.push(if self.cyclic_band_generators {
            CYCLIC_BAND_FLAG
        } else {
            0
        });
        out.extend_from_slice(&self.strands.to_le_bytes());
        out.extend_from_slice(&word_len.to_le_bytes());
        for letter in &self.word {
            out.extend_from_slice(&letter.to_le_bytes());
        }
        Ok(out)
    }

    /// Compact snapshot storage. Canonical keys and proof checkpoints continue
    /// to use `encode()` v0, so changing this cold codec cannot change identity.
    pub fn encode_storage(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let word_len: u32 = self
            .word
            .len()
            .try_into()
            .map_err(|_| "braid word is too long for the packed codec")?;
        let largest = if self.cyclic_band_generators {
            self.strands
        } else {
            self.strands.saturating_sub(1)
        };
        let mode = if largest <= 8 {
            NIBBLE_WORD_FLAG
        } else if largest <= 128 {
            BYTE_WORD_FLAG
        } else {
            0
        };
        let payload_len = match mode {
            NIBBLE_WORD_FLAG => self.word.len().div_ceil(2),
            BYTE_WORD_FLAG => self.word.len(),
            _ => self
                .word
                .len()
                .checked_mul(2)
                .ok_or("braid word too long")?,
        };
        let mut out = Vec::with_capacity(8 + payload_len);
        out.push(PACKED_MAGIC);
        out.push(
            if self.cyclic_band_generators {
                CYCLIC_BAND_FLAG
            } else {
                0
            } | mode,
        );
        write_varint(u64::from(self.strands), &mut out);
        write_varint(u64::from(word_len), &mut out);
        match mode {
            NIBBLE_WORD_FLAG => {
                for pair in self.word.chunks(2) {
                    let low = packed_letter(pair[0])? as u8;
                    let high = pair
                        .get(1)
                        .map(|letter| packed_letter(*letter).map(|code| (code as u8) << 4))
                        .transpose()?
                        .unwrap_or(0);
                    out.push(low | high);
                }
            }
            BYTE_WORD_FLAG => {
                for &letter in &self.word {
                    out.push(packed_letter(letter)? as u8);
                }
            }
            _ => {
                for &letter in &self.word {
                    out.extend_from_slice(&packed_letter(letter)?.to_le_bytes());
                }
            }
        }
        Ok(out)
    }

    pub fn mirrored(&self) -> Result<Self> {
        self.validate()?;
        let mirrored = Self {
            strands: self.strands,
            cyclic_band_generators: self.cyclic_band_generators,
            word: self.word.iter().map(|letter| -*letter).collect(),
        };
        mirrored.validate()?;
        Ok(mirrored)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err("braid representation is shorter than its header".into());
        }
        if bytes[..4] != MAGIC {
            return Err("bad braid representation magic".into());
        }
        if bytes[4] != 0 {
            return Err(format!("unsupported braid codec version {}", bytes[4]).into());
        }
        let flags = bytes[5];
        if flags & !CYCLIC_BAND_FLAG != 0 {
            return Err(format!("unknown braid representation flags {flags:#x}").into());
        }
        let strands = u16::from_le_bytes([bytes[6], bytes[7]]);
        let word_len = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let expected = HEADER_LEN
            .checked_add(
                word_len
                    .checked_mul(2)
                    .ok_or("braid word length overflow")?,
            )
            .ok_or("braid representation length overflow")?;
        if bytes.len() != expected {
            return Err(format!(
                "braid representation has {} bytes, expected {}",
                bytes.len(),
                expected
            )
            .into());
        }
        let word = bytes[HEADER_LEN..]
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let representation = Self {
            strands,
            cyclic_band_generators: flags & CYCLIC_BAND_FLAG != 0,
            word,
        };
        representation.validate()?;
        Ok(representation)
    }

    pub fn decode_storage(bytes: &[u8]) -> Result<Self> {
        if bytes.starts_with(&MAGIC) {
            return Self::decode(bytes);
        }
        if bytes.first() != Some(&PACKED_MAGIC) {
            return Err("bad or unsupported packed braid representation".into());
        }
        let flags = *bytes
            .get(1)
            .ok_or("packed braid representation has no flags")?;
        if flags & !(CYCLIC_BAND_FLAG | NIBBLE_WORD_FLAG | BYTE_WORD_FLAG) != 0
            || flags & (NIBBLE_WORD_FLAG | BYTE_WORD_FLAG) == (NIBBLE_WORD_FLAG | BYTE_WORD_FLAG)
        {
            return Err(format!("unknown packed braid representation flags {flags:#x}").into());
        }
        let mut cursor = 2_usize;
        let strands: u16 = read_varint(bytes, &mut cursor)?
            .try_into()
            .map_err(|_| "packed strand count exceeds u16")?;
        let word_len_u32: u32 = read_varint(bytes, &mut cursor)?
            .try_into()
            .map_err(|_| "packed word length exceeds u32")?;
        let word_len = word_len_u32 as usize;
        let mode = flags & (NIBBLE_WORD_FLAG | BYTE_WORD_FLAG);
        let largest = if flags & CYCLIC_BAND_FLAG != 0 {
            strands
        } else {
            strands.saturating_sub(1)
        };
        let canonical_mode = if largest <= 8 {
            NIBBLE_WORD_FLAG
        } else if largest <= 128 {
            BYTE_WORD_FLAG
        } else {
            0
        };
        if mode != canonical_mode {
            return Err("packed braid representation uses a noncanonical width".into());
        }
        let payload_len = match mode {
            NIBBLE_WORD_FLAG => word_len.div_ceil(2),
            BYTE_WORD_FLAG => word_len,
            _ => word_len
                .checked_mul(2)
                .ok_or("braid word length overflow")?,
        };
        let expected = cursor
            .checked_add(payload_len)
            .ok_or("braid representation length overflow")?;
        if bytes.len() != expected {
            return Err(format!(
                "packed braid representation has {} bytes, expected {}",
                bytes.len(),
                expected
            )
            .into());
        }
        let payload = &bytes[cursor..];
        if mode == NIBBLE_WORD_FLAG && word_len % 2 == 1 && payload.last().unwrap() >> 4 != 0 {
            return Err("packed braid representation has nonzero padding".into());
        }
        let codes: Vec<u16> = match mode {
            NIBBLE_WORD_FLAG => (0..word_len)
                .map(|index| {
                    let byte = payload[index / 2];
                    u16::from(if index % 2 == 0 {
                        byte & 0xf
                    } else {
                        byte >> 4
                    })
                })
                .collect(),
            BYTE_WORD_FLAG => payload.iter().map(|byte| u16::from(*byte)).collect(),
            _ => payload
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect(),
        };
        let word = codes
            .into_iter()
            .map(unpack_letter)
            .collect::<Result<_>>()?;
        let representation = Self {
            strands,
            cyclic_band_generators: flags & CYCLIC_BAND_FLAG != 0,
            word,
        };
        representation.validate()?;
        Ok(representation)
    }

    /// Canonical representative of the mirror orbit. Positive writhe is kept;
    /// negative writhe is reflected. At writhe zero, the lexicographically
    /// smaller origin-normalized word wins, with unmirrored winning exact ties.
    pub fn normalize(&self) -> Result<NormalizedRepresentation> {
        self.validate()?;
        let writhe: i64 = self.word.iter().map(|letter| i64::from(*letter)).sum();
        let mirrored_word = || self.word.iter().map(|letter| -*letter).collect::<Vec<_>>();
        let (mirrored, base_word, shift) = if writhe > 0 {
            (
                false,
                self.word.clone(),
                minimal_cyclic_rotation(&self.word),
            )
        } else if writhe < 0 {
            let word = mirrored_word();
            let shift = minimal_cyclic_rotation(&word);
            (true, word, shift)
        } else {
            let original_shift = minimal_cyclic_rotation(&self.word);
            let original = rotate_word(&self.word, original_shift);
            let reflected_word = mirrored_word();
            let reflected_shift = minimal_cyclic_rotation(&reflected_word);
            let reflected = rotate_word(&reflected_word, reflected_shift);
            if reflected < original {
                (true, reflected_word, reflected_shift)
            } else {
                (false, self.word.clone(), original_shift)
            }
        };
        let word = rotate_word(&base_word, shift);
        let representation = Self {
            strands: self.strands,
            cyclic_band_generators: self.cyclic_band_generators,
            word,
        };
        let encoded = representation.encode()?;
        let key = sha256_key(&encoded)?;
        Ok(NormalizedRepresentation {
            representation,
            witness: NormalizationWitness {
                mirrored,
                rotate_word_left: shift as u32,
            },
            encoded,
            key,
        })
    }

    /// Canonicalize only the cyclic word seam, preserving chirality. Proof
    /// replay uses this for a physical origin change inside one mirror chart.
    pub fn normalize_origin(&self) -> Result<NormalizedRepresentation> {
        self.validate()?;
        let shift = minimal_cyclic_rotation(&self.word);
        let representation = Self {
            strands: self.strands,
            cyclic_band_generators: self.cyclic_band_generators,
            word: rotate_word(&self.word, shift),
        };
        let encoded = representation.encode()?;
        let key = sha256_key(&encoded)?;
        Ok(NormalizedRepresentation {
            representation,
            witness: NormalizationWitness {
                mirrored: false,
                rotate_word_left: shift as u32,
            },
            encoded,
            key,
        })
    }

    pub fn is_normalized(&self) -> Result<bool> {
        let witness = self.normalize()?.witness;
        Ok(!witness.mirrored && witness.rotate_word_left == 0)
    }

    pub fn is_origin_normalized(&self) -> Result<bool> {
        Ok(self.normalize_origin()?.witness.rotate_word_left == 0)
    }

    /// Necessary parity for a one-component closure. Every braid letter is a
    /// transposition, while an `n`-cycle has sign `(-1)^(n-1)`.
    pub fn has_knot_closure_parity(&self) -> bool {
        self.word.len() % 2 == usize::from(self.strands.saturating_sub(1) % 2)
    }

    pub fn closure_component_count(&self) -> Result<u16> {
        self.validate()?;
        let n = usize::from(self.strands);
        let mut permutation: Vec<usize> = (0..n).collect();
        for letter in &self.word {
            let generator = usize::from(letter.unsigned_abs());
            let (left, right) = if generator == n {
                (n - 1, 0)
            } else {
                (generator - 1, generator)
            };
            permutation.swap(left, right);
        }
        let mut seen = vec![false; n];
        let mut components = 0_u16;
        for start in 0..n {
            if seen[start] {
                continue;
            }
            components += 1;
            let mut current = start;
            while !seen[current] {
                seen[current] = true;
                current = permutation[current];
            }
        }
        Ok(components)
    }

    pub fn is_knot_closure(&self) -> Result<bool> {
        if !self.has_knot_closure_parity() {
            return Ok(false);
        }
        Ok(self.closure_component_count()? == 1)
    }
}

impl NormalizationWitness {
    pub fn apply(&self, input: &BraidRepresentation) -> Result<BraidRepresentation> {
        input.validate()?;
        let reflected;
        let input_word = if self.mirrored {
            reflected = input.word.iter().map(|letter| -*letter).collect::<Vec<_>>();
            reflected.as_slice()
        } else {
            input.word.as_slice()
        };
        let len = input_word.len();
        let shift = if len == 0 {
            if self.rotate_word_left != 0 {
                return Err("an empty word can only have zero origin shift".into());
            }
            0
        } else {
            self.rotate_word_left as usize % len
        };
        let word = rotate_word(input_word, shift);
        Ok(BraidRepresentation {
            strands: input.strands,
            cyclic_band_generators: input.cyclic_band_generators,
            word,
        })
    }

    pub fn input_to_normalized_position(&self, position: u32, word_len: u32) -> Result<u32> {
        transport_position(position, word_len, self.rotate_word_left, false)
    }

    pub fn normalized_to_input_position(&self, position: u32, word_len: u32) -> Result<u32> {
        transport_position(position, word_len, self.rotate_word_left, true)
    }
}

fn packed_letter(letter: i16) -> Result<u16> {
    if letter == 0 {
        return Err("zero is not a braid letter".into());
    }
    let generator = letter.unsigned_abs();
    let base = generator
        .checked_sub(1)
        .and_then(|value| value.checked_mul(2))
        .ok_or("braid generator cannot be packed")?;
    Ok(base + u16::from(letter < 0))
}

fn write_varint(mut value: u64, out: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> Result<u64> {
    let start = *cursor;
    let mut value = 0_u64;
    for shift in (0..=63).step_by(7) {
        let byte = *bytes.get(*cursor).ok_or("truncated packed varint")?;
        *cursor += 1;
        if shift == 63 && byte & 0xfe != 0 {
            return Err("packed varint overflow".into());
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            let mut canonical = Vec::new();
            write_varint(value, &mut canonical);
            if canonical.as_slice() != &bytes[start..*cursor] {
                return Err("noncanonical packed varint".into());
            }
            return Ok(value);
        }
    }
    Err("packed varint overflow".into())
}

fn unpack_letter(code: u16) -> Result<i16> {
    let generator_u16 = code / 2 + 1;
    let generator: i16 = generator_u16
        .try_into()
        .map_err(|_| "packed braid generator exceeds i16")?;
    Ok(if code & 1 == 0 { generator } else { -generator })
}

/// Semantic action independent of RF Knots' capacity-dependent flat policy id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticAction {
    Reduce {
        position: u32,
    },
    Commute {
        position: u32,
    },
    Braid {
        position: u32,
    },
    Insert {
        position: u32,
        generator: u16,
        sign: i8,
    },
    Destabilize,
    StabilizePositive,
    StabilizeNegative,
    /// Markov stabilization at an exact word gap. This proof-capable form is
    /// needed to invert DESTABILIZE without losing the chosen linear origin.
    StabilizeAt {
        position: u32,
        sign: i8,
    },
    Pass,
    CrossingChange {
        position: u32,
    },
    /// Deterministic zero-cost theorem macro, legal only when the ordinary
    /// Artin closure is descending, or its ascending mirror, under
    /// `descending_crossing_changes_v0`.
    DescendingCollapse,
    /// Zero-cost theorem macro backed by a content-addressed labelled planar
    /// RI/RII/RIII certificate stored beside its graph edge.
    PlanarCertificateCollapse,
}

impl SemanticAction {
    const KIND_BITS: u32 = 4;
    const POSITION_BITS: u32 = 32;
    const GENERATOR_BITS: u32 = 16;
    const POSITION_SHIFT: u32 = Self::KIND_BITS;
    const GENERATOR_SHIFT: u32 = Self::POSITION_SHIFT + Self::POSITION_BITS;
    const NEGATIVE_SHIFT: u32 = Self::GENERATOR_SHIFT + Self::GENERATOR_BITS;
    const USED_BITS: u32 = Self::NEGATIVE_SHIFT + 1;

    pub fn encode_u63(self) -> Result<u64> {
        let (kind, position, generator, negative): (u8, u32, u16, bool) = match self {
            Self::Reduce { position } => (0, position, 0, false),
            Self::Commute { position } => (1, position, 0, false),
            Self::Braid { position } => (2, position, 0, false),
            Self::Insert {
                position,
                generator,
                sign,
            } => {
                if generator == 0 {
                    return Err("INSERT generator must be positive".into());
                }
                if sign != 1 && sign != -1 {
                    return Err("INSERT sign must be +1 or -1".into());
                }
                (3, position, generator, sign < 0)
            }
            Self::Destabilize => (4, 0, 0, false),
            Self::StabilizePositive => (5, 0, 0, false),
            Self::StabilizeNegative => (6, 0, 0, false),
            Self::Pass => (7, 0, 0, false),
            Self::CrossingChange { position } => (8, position, 0, false),
            Self::StabilizeAt { position, sign } => {
                if sign != 1 && sign != -1 {
                    return Err("STABILIZE_AT sign must be +1 or -1".into());
                }
                (9, position, 0, sign < 0)
            }
            Self::DescendingCollapse => (10, 0, 0, false),
            Self::PlanarCertificateCollapse => (11, 0, 0, false),
        };
        Ok(u64::from(kind)
            | (u64::from(position) << Self::POSITION_SHIFT)
            | (u64::from(generator) << Self::GENERATOR_SHIFT)
            | (u64::from(negative) << Self::NEGATIVE_SHIFT))
    }

    pub fn decode_u63(encoded: u64) -> Result<Self> {
        if encoded >> Self::USED_BITS != 0 {
            return Err("semantic action has non-zero reserved bits".into());
        }
        let kind = (encoded & 0xf) as u8;
        let position = ((encoded >> Self::POSITION_SHIFT) & u64::from(u32::MAX)) as u32;
        let generator = ((encoded >> Self::GENERATOR_SHIFT) & u64::from(u16::MAX)) as u16;
        let negative = (encoded >> Self::NEGATIVE_SHIFT) & 1 != 0;
        let no_extra = || -> Result<()> {
            if generator != 0 || negative {
                Err("non-INSERT action has generator/sign payload".into())
            } else {
                Ok(())
            }
        };
        Ok(match kind {
            0 => {
                no_extra()?;
                Self::Reduce { position }
            }
            1 => {
                no_extra()?;
                Self::Commute { position }
            }
            2 => {
                no_extra()?;
                Self::Braid { position }
            }
            3 => {
                if generator == 0 {
                    return Err("INSERT action has generator zero".into());
                }
                Self::Insert {
                    position,
                    generator,
                    sign: if negative { -1 } else { 1 },
                }
            }
            4..=7 | 10..=11 => {
                if position != 0 {
                    return Err("global action has a position payload".into());
                }
                no_extra()?;
                match kind {
                    4 => Self::Destabilize,
                    5 => Self::StabilizePositive,
                    6 => Self::StabilizeNegative,
                    7 => Self::Pass,
                    10 => Self::DescendingCollapse,
                    11 => Self::PlanarCertificateCollapse,
                    _ => unreachable!(),
                }
            }
            8 => {
                no_extra()?;
                Self::CrossingChange { position }
            }
            9 => {
                if generator != 0 {
                    return Err("STABILIZE_AT has a generator payload".into());
                }
                Self::StabilizeAt {
                    position,
                    sign: if negative { -1 } else { 1 },
                }
            }
            _ => return Err(format!("unknown semantic action kind {}", kind).into()),
        })
    }

    pub fn input_to_normalized(self, witness: NormalizationWitness, word_len: u32) -> Result<Self> {
        let action = if witness.mirrored {
            self.mirrored()
        } else {
            self
        };
        action.transport(witness, word_len, false)
    }

    pub fn normalized_to_input(self, witness: NormalizationWitness, word_len: u32) -> Result<Self> {
        let action = self.transport(witness, word_len, true)?;
        Ok(if witness.mirrored {
            action.mirrored()
        } else {
            action
        })
    }

    pub fn mirrored(self) -> Self {
        match self {
            Self::Insert {
                position,
                generator,
                sign,
            } => Self::Insert {
                position,
                generator,
                sign: -sign,
            },
            Self::StabilizePositive => Self::StabilizeNegative,
            Self::StabilizeNegative => Self::StabilizePositive,
            Self::StabilizeAt { position, sign } => Self::StabilizeAt {
                position,
                sign: -sign,
            },
            other => other,
        }
    }

    fn transport(
        self,
        witness: NormalizationWitness,
        word_len: u32,
        inverse: bool,
    ) -> Result<Self> {
        let position = |p| {
            if inverse {
                witness.normalized_to_input_position(p, word_len)
            } else {
                witness.input_to_normalized_position(p, word_len)
            }
        };
        Ok(match self {
            Self::Reduce { position: p } => Self::Reduce {
                position: position(p)?,
            },
            Self::Commute { position: p } => Self::Commute {
                position: position(p)?,
            },
            Self::Braid { position: p } => Self::Braid {
                position: position(p)?,
            },
            Self::Insert {
                position: p,
                generator,
                sign,
            } => Self::Insert {
                position: position(p)?,
                generator,
                sign,
            },
            Self::CrossingChange { position: p } => Self::CrossingChange {
                position: position(p)?,
            },
            Self::StabilizeAt { position: p, sign } => {
                let p = if word_len == 0 {
                    if p != 0 {
                        return Err("empty-word STABILIZE_AT position must be zero".into());
                    }
                    0
                } else {
                    position(p % word_len)?
                };
                Self::StabilizeAt { position: p, sign }
            }
            global => global,
        })
    }

    pub fn is_crossing_change(self) -> bool {
        matches!(self, Self::CrossingChange { .. })
    }

    /// Independent list-based semantics for proof replay. There is no training
    /// capacity here: a representation is rejected only for mathematical or
    /// codec illegality, not because one policy head happened to use a small L/N.
    pub fn apply(self, input: &BraidRepresentation) -> Result<BraidRepresentation> {
        input.validate()?;
        let mut output = input.clone();
        let len = output.word.len();
        let cyclic_index = |position: u32| -> Result<usize> {
            if len == 0 || position as usize >= len {
                Err(format!("position {} is outside word length {}", position, len).into())
            } else {
                Ok(position as usize)
            }
        };
        match self {
            Self::Reduce { position } => {
                if len < 2 {
                    return Err("REDUCE needs two distinct word positions".into());
                }
                let left = cyclic_index(position)?;
                let right = (left + 1) % len;
                if output.word[left] != -output.word[right] {
                    return Err("REDUCE pair does not cancel".into());
                }
                for index in if left < right {
                    [right, left]
                } else {
                    [left, right]
                } {
                    output.word.remove(index);
                }
            }
            Self::Commute { position } => {
                if len < 2 {
                    return Err("COMMUTE needs two distinct word positions".into());
                }
                let left = cyclic_index(position)?;
                let right = (left + 1) % len;
                let a = output.word[left].unsigned_abs();
                let b = output.word[right].unsigned_abs();
                let distance = a.abs_diff(b);
                let legal = if output.cyclic_band_generators {
                    a != b && distance != 1 && distance != output.strands - 1
                } else {
                    distance >= 2
                };
                if !legal {
                    return Err("COMMUTE generators are not far apart".into());
                }
                output.word.swap(left, right);
            }
            Self::Braid { position } => {
                if len < 3 {
                    return Err("BRAID needs three distinct word positions".into());
                }
                let first = cyclic_index(position)?;
                let second = (first + 1) % len;
                let third = (first + 2) % len;
                let a = output.word[first];
                let b = output.word[second];
                let distance = a.unsigned_abs().abs_diff(b.unsigned_abs());
                let adjacent = distance == 1
                    || (output.cyclic_band_generators && distance == output.strands - 1);
                if output.word[third] != a || (a > 0) != (b > 0) || !adjacent {
                    return Err("BRAID triple is not a same-sign (a,b,a) relation".into());
                }
                output.word[first] = b;
                output.word[second] = a;
                output.word[third] = b;
            }
            Self::Insert {
                position,
                generator,
                sign,
            } => {
                if output.strands < 2 {
                    return Err("INSERT needs at least two strands".into());
                }
                let largest = if output.cyclic_band_generators {
                    output.strands
                } else {
                    output.strands - 1
                };
                if generator == 0
                    || generator > largest
                    || (generator == output.strands && output.strands < 3)
                {
                    return Err(format!("INSERT generator {} is invalid", generator).into());
                }
                if sign != 1 && sign != -1 {
                    return Err("INSERT sign must be +1 or -1".into());
                }
                let position = if len == 0 {
                    if position != 0 {
                        return Err("empty-word INSERT position must be zero".into());
                    }
                    0
                } else {
                    cyclic_index(position)?
                };
                let letter = i16::try_from(generator)? * i16::from(sign);
                output.word.splice(position..position, [letter, -letter]);
            }
            Self::Destabilize => {
                if output.strands < 2 {
                    return Err("DESTABILIZE needs at least two strands".into());
                }
                if output.cyclic_band_generators
                    && output
                        .word
                        .iter()
                        .any(|letter| letter.unsigned_abs() == output.strands)
                {
                    return Err("DESTABILIZE is blocked by a cyclic seam generator".into());
                }
                let top = output.strands - 1;
                let positions: Vec<_> = output
                    .word
                    .iter()
                    .enumerate()
                    .filter_map(|(index, letter)| (letter.unsigned_abs() == top).then_some(index))
                    .collect();
                if positions.len() != 1 {
                    return Err(format!(
                        "DESTABILIZE needs one top generator, found {}",
                        positions.len()
                    )
                    .into());
                }
                output.word.remove(positions[0]);
                output.strands -= 1;
            }
            Self::StabilizePositive | Self::StabilizeNegative => {
                if output.strands >= i16::MAX as u16 {
                    return Err("STABILIZE exceeds the v0 strand capacity".into());
                }
                if output.cyclic_band_generators
                    && output
                        .word
                        .iter()
                        .any(|letter| letter.unsigned_abs() == output.strands)
                {
                    return Err("STABILIZE is blocked by a cyclic seam generator".into());
                }
                let letter = i16::try_from(output.strands)?;
                output
                    .word
                    .push(if matches!(self, Self::StabilizePositive) {
                        letter
                    } else {
                        -letter
                    });
                output.strands += 1;
            }
            Self::StabilizeAt { position, sign } => {
                if output.strands >= i16::MAX as u16 {
                    return Err("STABILIZE_AT exceeds the v0 strand capacity".into());
                }
                if output.cyclic_band_generators
                    && output
                        .word
                        .iter()
                        .any(|letter| letter.unsigned_abs() == output.strands)
                {
                    return Err("STABILIZE_AT is blocked by a cyclic seam generator".into());
                }
                if sign != 1 && sign != -1 {
                    return Err("STABILIZE_AT sign must be +1 or -1".into());
                }
                let position = usize::try_from(position)?;
                if position > len {
                    return Err(format!(
                        "STABILIZE_AT position {} is outside {} word gaps",
                        position,
                        len + 1
                    )
                    .into());
                }
                let letter = i16::try_from(output.strands)? * i16::from(sign);
                output.word.insert(position, letter);
                output.strands += 1;
            }
            Self::Pass => return Err("PASS is a controller action, not a proof primitive".into()),
            Self::CrossingChange { position } => {
                let position = cyclic_index(position)?;
                output.word[position] = -output.word[position];
            }
            Self::DescendingCollapse => {
                // Mirror-orbit canonicalization can transport a descending
                // diagram to its ascending mirror and origin normalization can
                // move the chosen base point. Existence of either monotone
                // cyclic traversal is invariant under both coordinate moves.
                if !has_monotone_traversal_v0(&output)? {
                    return Err(
                        "DESCENDING_COLLAPSE input is neither descending nor ascending".into(),
                    );
                }
                output = BraidRepresentation {
                    strands: 1,
                    cyclic_band_generators: false,
                    word: Vec::new(),
                };
            }
            Self::PlanarCertificateCollapse => {
                return Err("PLANAR_CERTIFICATE_COLLAPSE requires its edge sidecar".into())
            }
        }
        output.validate()?;
        Ok(output)
    }
}

/// Return the crossing indices that must be toggled to make an ordinary Artin
/// braid closure descending. Traversal v0 starts at top strand 0, follows each
/// strand downward, and closes bottom position `i` to top position `i`.
/// Positive sigma_i has the branch entering at i over; negative sigma_i has
/// the branch entering at i+1 over. Each crossing must first be met over.
pub fn descending_crossing_changes_v0(input: &BraidRepresentation) -> Result<Vec<u32>> {
    input.validate()?;
    if input.cyclic_band_generators {
        return Err("descending-v0 supports ordinary Artin generators only".into());
    }
    if !input.is_knot_closure()? {
        return Err("descending-v0 requires a one-component closure".into());
    }
    let strands = usize::from(input.strands);
    let mut labels: Vec<usize> = (0..strands).collect();
    let mut events = vec![Vec::<(u32, bool)>::new(); strands];
    for (crossing, &letter) in input.word.iter().enumerate() {
        let left = usize::from(letter.unsigned_abs() - 1);
        let right = left + 1;
        let left_label = labels[left];
        let right_label = labels[right];
        let left_over = letter > 0;
        let crossing: u32 = crossing.try_into()?;
        events[left_label].push((crossing, left_over));
        events[right_label].push((crossing, !left_over));
        labels.swap(left, right);
    }
    let mut bottom_position = vec![0_usize; strands];
    for (position, label) in labels.into_iter().enumerate() {
        bottom_position[label] = position;
    }
    let mut seen_crossing = vec![false; input.word.len()];
    let mut seen_label = vec![false; strands];
    let mut changes = Vec::new();
    let mut label = 0_usize;
    loop {
        if seen_label[label] {
            break;
        }
        seen_label[label] = true;
        for &(crossing, over) in &events[label] {
            let index = crossing as usize;
            if !seen_crossing[index] {
                seen_crossing[index] = true;
                if !over {
                    changes.push(crossing);
                }
            }
        }
        label = bottom_position[label];
    }
    if seen_label.iter().any(|seen| !seen) || seen_crossing.iter().any(|seen| !seen) {
        return Err("descending-v0 traversal did not cover the knot diagram".into());
    }
    Ok(changes)
}

/// Verify that some exact cyclic base point makes the one-component traversal
/// descending (all crossings first met over) or ascending (all first met
/// under). This is the coordinate-invariant terminal predicate used after the
/// mirror/origin quotient transports a descending certificate.
fn has_monotone_traversal_v0(input: &BraidRepresentation) -> Result<bool> {
    input.validate()?;
    if input.cyclic_band_generators {
        return Err("descending-v0 supports ordinary Artin generators only".into());
    }
    if !input.is_knot_closure()? {
        return Err("descending-v0 requires a one-component closure".into());
    }
    if input.word.is_empty() {
        return Ok(true);
    }
    let strands = usize::from(input.strands);
    let mut labels: Vec<usize> = (0..strands).collect();
    let mut events = vec![Vec::<(usize, bool)>::new(); strands];
    for (crossing, &letter) in input.word.iter().enumerate() {
        let left = usize::from(letter.unsigned_abs() - 1);
        let right = left + 1;
        let left_label = labels[left];
        let right_label = labels[right];
        let left_over = letter > 0;
        events[left_label].push((crossing, left_over));
        events[right_label].push((crossing, !left_over));
        labels.swap(left, right);
    }
    let mut bottom_position = vec![0_usize; strands];
    for (position, label) in labels.into_iter().enumerate() {
        bottom_position[label] = position;
    }
    let mut traversal = Vec::with_capacity(input.word.len() * 2);
    let mut seen_label = vec![false; strands];
    let mut label = 0_usize;
    while !seen_label[label] {
        seen_label[label] = true;
        traversal.extend_from_slice(&events[label]);
        label = bottom_position[label];
    }
    if seen_label.iter().any(|seen| !seen) || traversal.len() != input.word.len() * 2 {
        return Err("descending-v0 traversal did not cover the knot diagram".into());
    }
    for start in 0..traversal.len() {
        let mut seen = vec![false; input.word.len()];
        let mut first_over = None;
        let mut monotone = true;
        for offset in 0..traversal.len() {
            let (crossing, over) = traversal[(start + offset) % traversal.len()];
            if !seen[crossing] {
                seen[crossing] = true;
                if let Some(expected) = first_over {
                    monotone &= over == expected;
                } else {
                    first_over = Some(over);
                }
            }
        }
        if monotone {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofProgram {
    pub actions: Vec<SemanticAction>,
}

impl ProofProgram {
    pub fn encode(&self) -> Result<Vec<u8>> {
        let count: u32 = self
            .actions
            .len()
            .try_into()
            .map_err(|_| "proof program has too many actions")?;
        let mut out = Vec::with_capacity(PROGRAM_HEADER_LEN + self.actions.len() * 8);
        out.extend_from_slice(&PROGRAM_MAGIC);
        out.push(0);
        out.extend_from_slice(&[0, 0, 0]);
        out.extend_from_slice(&count.to_le_bytes());
        for action in &self.actions {
            out.extend_from_slice(&action.encode_u63()?.to_le_bytes());
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < PROGRAM_HEADER_LEN {
            return Err("proof program is shorter than its header".into());
        }
        if bytes[..4] != PROGRAM_MAGIC {
            return Err("bad proof program magic".into());
        }
        if bytes[4] != 0 {
            return Err(format!("unsupported proof program version {}", bytes[4]).into());
        }
        if bytes[5..8] != [0, 0, 0] {
            return Err("proof program reserved header bytes are non-zero".into());
        }
        let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let expected = PROGRAM_HEADER_LEN
            .checked_add(
                count
                    .checked_mul(8)
                    .ok_or("proof program length overflow")?,
            )
            .ok_or("proof program length overflow")?;
        if bytes.len() != expected {
            return Err(format!(
                "proof program has {} bytes, expected {}",
                bytes.len(),
                expected
            )
            .into());
        }
        let actions = bytes[PROGRAM_HEADER_LEN..]
            .chunks_exact(8)
            .map(|chunk| SemanticAction::decode_u63(u64::from_le_bytes(chunk.try_into().unwrap())))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { actions })
    }

    pub fn cc_cost(&self) -> u32 {
        self.actions
            .iter()
            .filter(|action| action.is_crossing_change())
            .count() as u32
    }

    pub fn replay(&self, source: &BraidRepresentation) -> Result<BraidRepresentation> {
        if self.actions.is_empty() {
            return Err("proof program is empty".into());
        }
        let mut state = source.clone();
        for (index, action) in self.actions.iter().copied().enumerate() {
            state = action
                .apply(&state)
                .map_err(|error| format!("action {} {:?}: {}", index, action, error))?;
        }
        Ok(state)
    }
}

/// A compact edge program that can replay canonical origin changes between
/// semantic moves. An origin change has zero CC cost and is accepted only when
/// its stored shift exactly matches the independent normalizer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofInstruction {
    Action(SemanticAction),
    NormalizeOrigin { rotate_word_left: u32 },
    RotateOriginLeft { amount: u32 },
    MirrorOrbit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointedProofProgram {
    pub instructions: Vec<ProofInstruction>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticReplayStep {
    pub instruction_index: u32,
    pub state: BraidRepresentation,
    pub action: SemanticAction,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstructionReplayStep {
    pub instruction_index: u32,
    pub before: BraidRepresentation,
    pub instruction: ProofInstruction,
    pub after: BraidRepresentation,
}

/// A coordinate-independent proof template plus the torus coordinate at which
/// it is instantiated by one edge.  `anchor_y` is the position of the first
/// position-bearing semantic action.  Every stored position in `template` is
/// relative to that anchor in the action's current cyclic word chart.
/// `anchor_x` records the generator at that first operation for indexing and
/// pattern search; replay derives horizontal action semantics from the state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnchoredProofProgram {
    pub template: CheckpointedProofProgram,
    pub anchor_x: u16,
    pub anchor_y: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterializedProofProgram {
    /// A physical program with no `MIRROR_ORBIT` instructions.
    pub program: CheckpointedProofProgram,
    /// Chirality of the physical target relative to the graph target chart.
    pub target_mirrored: bool,
    pub target: BraidRepresentation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvertedProofProgram {
    /// Exact endpoint reached by the forward program and source of `program`.
    pub source: BraidRepresentation,
    /// State from which the forward program started and target of `program`.
    pub target: BraidRepresentation,
    /// State-aware inverse with a semantic first instruction.
    pub program: CheckpointedProofProgram,
}

impl CheckpointedProofProgram {
    pub fn contains_planar_certificate(&self) -> bool {
        self.instructions.iter().any(|instruction| {
            matches!(
                instruction,
                ProofInstruction::Action(SemanticAction::PlanarCertificateCollapse)
            )
        })
    }

    pub const VERSION: u32 = 1;

    pub fn encode(&self) -> Result<Vec<u8>> {
        let count: u32 = self
            .instructions
            .len()
            .try_into()
            .map_err(|_| "checkpointed proof program has too many instructions")?;
        let mut out = Vec::with_capacity(PROGRAM_HEADER_LEN + self.instructions.len() * 8);
        out.extend_from_slice(&PROGRAM_MAGIC);
        out.push(Self::VERSION as u8);
        out.extend_from_slice(&[0, 0, 0]);
        out.extend_from_slice(&count.to_le_bytes());
        for instruction in &self.instructions {
            let encoded = match instruction {
                ProofInstruction::Action(action) => action.encode_u63()?,
                ProofInstruction::NormalizeOrigin { rotate_word_left } => {
                    NORMALIZE_ORIGIN_TAG
                        | (NORMALIZE_ORIGIN_OPCODE << ORBIT_OPCODE_SHIFT)
                        | u64::from(*rotate_word_left)
                }
                ProofInstruction::RotateOriginLeft { amount } => {
                    NORMALIZE_ORIGIN_TAG
                        | (ROTATE_ORIGIN_OPCODE << ORBIT_OPCODE_SHIFT)
                        | u64::from(*amount)
                }
                ProofInstruction::MirrorOrbit => {
                    NORMALIZE_ORIGIN_TAG | (MIRROR_ORBIT_OPCODE << ORBIT_OPCODE_SHIFT)
                }
            };
            out.extend_from_slice(&encoded.to_le_bytes());
        }
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < PROGRAM_HEADER_LEN {
            return Err("checkpointed proof program is shorter than its header".into());
        }
        if bytes[..4] != PROGRAM_MAGIC {
            return Err("bad checkpointed proof program magic".into());
        }
        if bytes[4] != Self::VERSION as u8 {
            return Err(format!("unsupported checkpointed program version {}", bytes[4]).into());
        }
        if bytes[5..8] != [0, 0, 0] {
            return Err("checkpointed proof program reserved header bytes are non-zero".into());
        }
        let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let expected = PROGRAM_HEADER_LEN
            .checked_add(
                count
                    .checked_mul(8)
                    .ok_or("checkpointed proof program length overflow")?,
            )
            .ok_or("checkpointed proof program length overflow")?;
        if bytes.len() != expected {
            return Err(format!(
                "checkpointed proof program has {} bytes, expected {}",
                bytes.len(),
                expected
            )
            .into());
        }
        let instructions = bytes[PROGRAM_HEADER_LEN..]
            .chunks_exact(8)
            .map(|chunk| {
                let encoded = u64::from_le_bytes(chunk.try_into().unwrap());
                if encoded & NORMALIZE_ORIGIN_TAG == 0 {
                    Ok(ProofInstruction::Action(SemanticAction::decode_u63(
                        encoded,
                    )?))
                } else {
                    if encoded & !(NORMALIZE_ORIGIN_TAG | ORBIT_OPCODE_MASK | u64::from(u32::MAX))
                        != 0
                    {
                        return Err("orbit instruction has non-zero reserved bits".into());
                    }
                    let opcode = (encoded & ORBIT_OPCODE_MASK) >> ORBIT_OPCODE_SHIFT;
                    let payload = encoded as u32;
                    Ok(match opcode {
                        NORMALIZE_ORIGIN_OPCODE => ProofInstruction::NormalizeOrigin {
                            rotate_word_left: payload,
                        },
                        ROTATE_ORIGIN_OPCODE => {
                            ProofInstruction::RotateOriginLeft { amount: payload }
                        }
                        MIRROR_ORBIT_OPCODE if payload == 0 => ProofInstruction::MirrorOrbit,
                        MIRROR_ORBIT_OPCODE => {
                            return Err("MIRROR_ORBIT has a non-zero payload".into())
                        }
                        _ => return Err("unknown orbit instruction opcode".into()),
                    })
                }
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { instructions })
    }

    pub fn first_action(&self) -> Result<SemanticAction> {
        match self.instructions.first() {
            Some(ProofInstruction::Action(action)) => Ok(*action),
            Some(
                ProofInstruction::NormalizeOrigin { .. }
                | ProofInstruction::RotateOriginLeft { .. }
                | ProofInstruction::MirrorOrbit,
            ) => {
                let first_semantic = self.instructions.iter().find_map(|instruction| {
                    if let ProofInstruction::Action(action) = instruction {
                        Some(*action)
                    } else {
                        None
                    }
                });
                if first_semantic == Some(SemanticAction::PlanarCertificateCollapse) {
                    Ok(SemanticAction::PlanarCertificateCollapse)
                } else {
                    Err("edge program cannot start with an origin normalization".into())
                }
            }
            None => Err("checkpointed proof program is empty".into()),
        }
    }

    pub fn cc_cost(&self) -> u32 {
        self.instructions
            .iter()
            .filter(|instruction| {
                matches!(instruction, ProofInstruction::Action(action) if action.is_crossing_change())
            })
            .count() as u32
    }

    pub fn replay(&self, source: &BraidRepresentation) -> Result<BraidRepresentation> {
        if self.instructions.is_empty() {
            return Err("checkpointed proof program is empty".into());
        }
        let mut state = source.clone();
        for (index, instruction) in self.instructions.iter().copied().enumerate() {
            state = apply_proof_instruction(&state, instruction, index)?;
        }
        Ok(state)
    }

    /// Replay while retaining the exact physical state immediately before each
    /// semantic action. Coordinate-only instructions are applied but do not
    /// become training labels.
    pub fn semantic_replay_steps(
        &self,
        source: &BraidRepresentation,
    ) -> Result<Vec<SemanticReplayStep>> {
        if self.instructions.is_empty() {
            return Err("checkpointed proof program is empty".into());
        }
        let mut state = source.clone();
        let mut steps = Vec::new();
        for (index, instruction) in self.instructions.iter().copied().enumerate() {
            if let ProofInstruction::Action(action) = instruction {
                steps.push(SemanticReplayStep {
                    instruction_index: index.try_into()?,
                    state: state.clone(),
                    action,
                });
            }
            state = apply_proof_instruction(&state, instruction, index)?;
        }
        Ok(steps)
    }

    /// Replay every instruction while retaining exact before/after states.
    /// This is used by derived metric-learning sidecars; it does not weaken
    /// proof validation or admit any new instruction semantics.
    pub fn instruction_replay_steps(
        &self,
        source: &BraidRepresentation,
    ) -> Result<Vec<InstructionReplayStep>> {
        if self.instructions.is_empty() {
            return Err("checkpointed proof program is empty".into());
        }
        let mut state = source.clone();
        let mut steps = Vec::with_capacity(self.instructions.len());
        for (index, instruction) in self.instructions.iter().copied().enumerate() {
            let before = state.clone();
            state = apply_proof_instruction(&state, instruction, index)?;
            steps.push(InstructionReplayStep {
                instruction_index: index.try_into()?,
                before,
                instruction,
                after: state.clone(),
            });
        }
        Ok(steps)
    }

    /// Factor the position of the first local operation out of the immutable
    /// program and into an edge binding.  The anchor is held fixed as an
    /// integer and reduced modulo each later action's current coordinate
    /// domain, which makes insertion/reduction and origin changes replayable.
    pub fn anchor_template(&self, source: &BraidRepresentation) -> Result<AnchoredProofProgram> {
        if self.instructions.is_empty() {
            return Err("cannot anchor an empty proof program".into());
        }
        source.validate()?;
        let mut state = source.clone();
        let mut anchor_y = None;
        let mut anchor_x = 0_u16;
        let mut instructions = Vec::with_capacity(self.instructions.len());
        for (index, instruction) in self.instructions.iter().copied().enumerate() {
            let template_instruction = match instruction {
                ProofInstruction::Action(action) => {
                    let template_action = if let Some(position) = action_position(action) {
                        let domain = action_position_domain(action, &state)?;
                        let base = if let Some(base) = anchor_y {
                            base
                        } else {
                            anchor_x = action_anchor_generator(action, &state)?;
                            anchor_y = Some(position);
                            position
                        };
                        let relative = cyclic_difference(position, base, domain)?;
                        ProofInstruction::Action(action_with_position(action, relative)?)
                    } else {
                        instruction
                    };
                    state = action.apply(&state).map_err(|error| {
                        format!(
                            "cannot anchor instruction {} {:?}: {}",
                            index, action, error
                        )
                    })?;
                    template_action
                }
                _ => {
                    state = apply_proof_instruction(&state, instruction, index)?;
                    instruction
                }
            };
            instructions.push(template_instruction);
        }
        let anchored = AnchoredProofProgram {
            template: CheckpointedProofProgram { instructions },
            anchor_x,
            anchor_y: anchor_y.unwrap_or(0),
        };
        if anchored.materialize(source)? != *self {
            return Err("anchored program does not materialize to its exact source program".into());
        }
        Ok(anchored)
    }

    /// Build an exact state-aware inverse. Decreasing primitives need their
    /// replayed pre-state to recover insertion signs and cyclic coordinates.
    pub fn invert_exact(&self, source: &BraidRepresentation) -> Result<InvertedProofProgram> {
        if self.instructions.is_empty() {
            return Err("cannot invert an empty checkpointed program".into());
        }
        source.validate()?;
        let mut states = Vec::with_capacity(self.instructions.len() + 1);
        states.push(source.clone());
        for (index, instruction) in self.instructions.iter().copied().enumerate() {
            let next = apply_proof_instruction(states.last().unwrap(), instruction, index)?;
            states.push(next);
        }
        let forward_target = states.last().unwrap().clone();
        let mut inverse = Vec::new();
        for index in (0..self.instructions.len()).rev() {
            let chunk = invert_proof_instruction(
                &states[index],
                self.instructions[index],
                &states[index + 1],
            )?;
            let replayed = replay_instruction_slice(&states[index + 1], &chunk)?;
            if replayed != states[index] {
                return Err(format!("inverse instruction {} failed exact replay", index).into());
            }
            inverse.extend(chunk);
        }
        let program = CheckpointedProofProgram {
            instructions: inverse,
        };
        if program.replay(&forward_target)? != *source {
            return Err("raw inverse program does not reproduce its source".into());
        }
        if program.cc_cost() != self.cc_cost() {
            return Err("inverse program changed crossing-change cost".into());
        }
        Ok(InvertedProofProgram {
            source: forward_target,
            target: source.clone(),
            program,
        })
    }

    /// Canonicalize an exact inverse for use as a standalone graph edge. Any
    /// leading coordinate changes are conjugated past the first semantic
    /// action so the stored edge exposes a semantic `first_action`.
    pub fn invert_edge(&self, source: &BraidRepresentation) -> Result<InvertedProofProgram> {
        let inverted = self.invert_exact(source)?;
        let program = semantic_first_inverse(inverted.program, &inverted.source)?;
        if program.replay(&inverted.source)? != *source {
            return Err("canonical inverse program does not reproduce its target".into());
        }
        if program.cc_cost() != self.cc_cost() {
            return Err("inverse program changed crossing-change cost".into());
        }
        program.first_action()?;
        Ok(InvertedProofProgram {
            source: inverted.source,
            target: source.clone(),
            program,
        })
    }

    /// Conjugate any leading coordinate-only instructions through the first
    /// semantic action while preserving the exact replay endpoint.
    pub fn canonicalize_semantic_first(
        &self,
        source: &BraidRepresentation,
    ) -> Result<CheckpointedProofProgram> {
        let target = self.replay(source)?;
        let program = semantic_first_inverse(self.clone(), source)?;
        if program.replay(source)? != target || program.cc_cost() != self.cc_cost() {
            return Err("semantic-first canonicalization changed the proof program".into());
        }
        program.first_action()?;
        Ok(program)
    }

    /// Remove quotient-only mirror boundaries from a program for one concrete
    /// input chirality. The parity bit is carried to the target and every
    /// semantic action is mirrored when the active chart is reflected.
    pub fn materialize_for_chirality(
        &self,
        graph_source: &BraidRepresentation,
        source_mirrored: bool,
    ) -> Result<MaterializedProofProgram> {
        graph_source.validate()?;
        let mut graph_state = graph_source.clone();
        let mut concrete_state = if source_mirrored {
            graph_source.mirrored()?
        } else {
            graph_source.clone()
        };
        let mut mirrored = source_mirrored;
        let mut instructions = Vec::new();

        for (index, instruction) in self.instructions.iter().copied().enumerate() {
            match instruction {
                ProofInstruction::Action(action) => {
                    graph_state = action.apply(&graph_state).map_err(|error| {
                        format!("orbit instruction {} action {:?}: {}", index, action, error)
                    })?;
                    let concrete_action = if mirrored { action.mirrored() } else { action };
                    concrete_state = concrete_action.apply(&concrete_state).map_err(|error| {
                        format!(
                            "materialized instruction {} action {:?}: {}",
                            index, concrete_action, error
                        )
                    })?;
                    instructions.push(ProofInstruction::Action(concrete_action));
                }
                ProofInstruction::NormalizeOrigin { rotate_word_left } => {
                    let normalized = graph_state.normalize_origin()?;
                    if normalized.witness.rotate_word_left != rotate_word_left {
                        return Err(format!(
                            "orbit instruction {} states origin shift {}, recomputed {}",
                            index, rotate_word_left, normalized.witness.rotate_word_left
                        )
                        .into());
                    }
                    graph_state = normalized.representation;
                    concrete_state = rotate_representation(&concrete_state, rotate_word_left)?;
                    instructions.push(ProofInstruction::RotateOriginLeft {
                        amount: rotate_word_left,
                    });
                }
                ProofInstruction::RotateOriginLeft { amount } => {
                    graph_state = rotate_representation(&graph_state, amount)?;
                    concrete_state = rotate_representation(&concrete_state, amount)?;
                    instructions.push(ProofInstruction::RotateOriginLeft { amount });
                }
                ProofInstruction::MirrorOrbit => {
                    graph_state = graph_state.mirrored()?;
                    mirrored = !mirrored;
                }
            }
            let expected_concrete = if mirrored {
                graph_state.mirrored()?
            } else {
                graph_state.clone()
            };
            if concrete_state != expected_concrete {
                return Err(format!(
                    "materialized program diverges from its orbit after instruction {}",
                    index
                )
                .into());
            }
        }
        let program = CheckpointedProofProgram { instructions };
        if program
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, ProofInstruction::MirrorOrbit))
        {
            return Err("internal error: concrete program contains MIRROR_ORBIT".into());
        }
        if program.replay(&if source_mirrored {
            graph_source.mirrored()?
        } else {
            graph_source.clone()
        })? != concrete_state
        {
            return Err("materialized program failed its independent replay".into());
        }
        Ok(MaterializedProofProgram {
            program,
            target_mirrored: mirrored,
            target: concrete_state,
        })
    }
}

impl AnchoredProofProgram {
    /// Instantiate a relative template in the concrete chart selected by an
    /// edge.  Replay while materializing makes coordinate domains exact even
    /// when earlier actions change the word length.
    pub fn materialize(&self, source: &BraidRepresentation) -> Result<CheckpointedProofProgram> {
        source.validate()?;
        let mut state = source.clone();
        let mut instructions = Vec::with_capacity(self.template.instructions.len());
        let mut saw_anchor = false;
        for (index, instruction) in self.template.instructions.iter().copied().enumerate() {
            let concrete = match instruction {
                ProofInstruction::Action(action) => {
                    if let Some(relative) = action_position(action) {
                        let domain = action_position_domain(action, &state)?;
                        let first_anchor = !saw_anchor;
                        if first_anchor {
                            if relative != 0 || self.anchor_y >= domain {
                                return Err(
                                    "program binding is not in canonical anchor form".into()
                                );
                            }
                            saw_anchor = true;
                        }
                        let position = cyclic_sum(self.anchor_y, relative, domain)?;
                        let action = action_with_position(action, position)?;
                        if first_anchor {
                            let generator = action_anchor_generator(action, &state)?;
                            if generator != self.anchor_x {
                                return Err(
                                    "program anchor_x does not match its first operation".into()
                                );
                            }
                        }
                        ProofInstruction::Action(action)
                    } else {
                        instruction
                    }
                }
                _ => instruction,
            };
            state = apply_proof_instruction(&state, concrete, index)?;
            instructions.push(concrete);
        }
        if !saw_anchor && (self.anchor_x != 0 || self.anchor_y != 0) {
            return Err("coordinate-free program has a non-zero anchor".into());
        }
        Ok(CheckpointedProofProgram { instructions })
    }
}

fn action_position(action: SemanticAction) -> Option<u32> {
    match action {
        SemanticAction::Reduce { position }
        | SemanticAction::Commute { position }
        | SemanticAction::Braid { position }
        | SemanticAction::Insert { position, .. }
        | SemanticAction::StabilizeAt { position, .. }
        | SemanticAction::CrossingChange { position } => Some(position),
        SemanticAction::Destabilize
        | SemanticAction::StabilizePositive
        | SemanticAction::StabilizeNegative
        | SemanticAction::Pass
        | SemanticAction::DescendingCollapse
        | SemanticAction::PlanarCertificateCollapse => None,
    }
}

fn action_with_position(action: SemanticAction, position: u32) -> Result<SemanticAction> {
    Ok(match action {
        SemanticAction::Reduce { .. } => SemanticAction::Reduce { position },
        SemanticAction::Commute { .. } => SemanticAction::Commute { position },
        SemanticAction::Braid { .. } => SemanticAction::Braid { position },
        SemanticAction::Insert {
            generator, sign, ..
        } => SemanticAction::Insert {
            position,
            generator,
            sign,
        },
        SemanticAction::StabilizeAt { sign, .. } => SemanticAction::StabilizeAt { position, sign },
        SemanticAction::CrossingChange { .. } => SemanticAction::CrossingChange { position },
        _ => return Err("global semantic action has no anchor position".into()),
    })
}

fn action_position_domain(action: SemanticAction, state: &BraidRepresentation) -> Result<u32> {
    let len: u32 = state
        .word
        .len()
        .try_into()
        .map_err(|_| "word is too long for an anchored program")?;
    match action {
        SemanticAction::StabilizeAt { position, .. } => {
            let domain = len.checked_add(1).ok_or("word-gap domain overflow")?;
            if position >= domain {
                return Err("STABILIZE_AT anchor position is outside word gaps".into());
            }
            Ok(domain)
        }
        _ if action_position(action).is_some() => {
            if len == 0 {
                if matches!(action, SemanticAction::Insert { position: 0, .. }) {
                    Ok(1)
                } else {
                    Err("position-bearing action has an empty coordinate domain".into())
                }
            } else {
                let position = action_position(action).unwrap();
                if position >= len {
                    return Err("action anchor position is outside the cyclic word".into());
                }
                Ok(len)
            }
        }
        _ => Err("global semantic action has no coordinate domain".into()),
    }
}

fn action_anchor_generator(action: SemanticAction, state: &BraidRepresentation) -> Result<u16> {
    match action {
        SemanticAction::Insert { generator, .. } => Ok(generator),
        SemanticAction::StabilizeAt { .. } => Ok(state.strands),
        _ => {
            let position = action_position(action).ok_or("action has no anchor generator")?;
            state
                .word
                .get(position as usize)
                .map(|letter| letter.unsigned_abs())
                .ok_or_else(|| "anchor generator position is outside the word".into())
        }
    }
}

fn cyclic_difference(position: u32, anchor: u32, domain: u32) -> Result<u32> {
    if domain == 0 || position >= domain {
        return Err("invalid anchored coordinate domain".into());
    }
    Ok((position + domain - anchor % domain) % domain)
}

fn cyclic_sum(anchor: u32, relative: u32, domain: u32) -> Result<u32> {
    if domain == 0 || relative >= domain {
        return Err("invalid relative program coordinate".into());
    }
    Ok((anchor % domain + relative) % domain)
}

fn apply_proof_instruction(
    state: &BraidRepresentation,
    instruction: ProofInstruction,
    index: usize,
) -> Result<BraidRepresentation> {
    match instruction {
        ProofInstruction::Action(action) => action.apply(state).map_err(|error| {
            format!("instruction {} action {:?}: {}", index, action, error).into()
        }),
        ProofInstruction::NormalizeOrigin { rotate_word_left } => {
            let normalized = state.normalize_origin()?;
            if normalized.witness.rotate_word_left != rotate_word_left {
                return Err(format!(
                    "instruction {} states origin shift {}, recomputed {}",
                    index, rotate_word_left, normalized.witness.rotate_word_left
                )
                .into());
            }
            Ok(normalized.representation)
        }
        ProofInstruction::RotateOriginLeft { amount } => rotate_representation(state, amount)
            .map_err(|error| format!("instruction {}: {}", index, error).into()),
        ProofInstruction::MirrorOrbit => state.mirrored(),
    }
}

fn replay_instruction_slice(
    source: &BraidRepresentation,
    instructions: &[ProofInstruction],
) -> Result<BraidRepresentation> {
    let mut state = source.clone();
    for (index, instruction) in instructions.iter().copied().enumerate() {
        state = apply_proof_instruction(&state, instruction, index)?;
    }
    Ok(state)
}

fn invert_proof_instruction(
    before: &BraidRepresentation,
    instruction: ProofInstruction,
    after: &BraidRepresentation,
) -> Result<Vec<ProofInstruction>> {
    if apply_proof_instruction(before, instruction, 0)? != *after {
        return Err("inverse builder received a discontinuous checkpoint".into());
    }
    Ok(match instruction {
        ProofInstruction::Action(action) => invert_semantic_action(before, action, after)?,
        ProofInstruction::NormalizeOrigin { rotate_word_left }
        | ProofInstruction::RotateOriginLeft {
            amount: rotate_word_left,
        } => {
            let len: u32 = before
                .word
                .len()
                .try_into()
                .map_err(|_| "word length does not fit inverse rotation")?;
            let amount = if len == 0 {
                0
            } else {
                (len - (rotate_word_left % len)) % len
            };
            vec![ProofInstruction::RotateOriginLeft { amount }]
        }
        ProofInstruction::MirrorOrbit => vec![ProofInstruction::MirrorOrbit],
    })
}

fn invert_semantic_action(
    before: &BraidRepresentation,
    action: SemanticAction,
    after: &BraidRepresentation,
) -> Result<Vec<ProofInstruction>> {
    let exact = |inverse: SemanticAction| -> Result<Vec<ProofInstruction>> {
        if inverse.apply(after)? != *before {
            return Err(format!(
                "semantic inverse {:?} does not restore its checkpoint",
                inverse
            )
            .into());
        }
        Ok(vec![ProofInstruction::Action(inverse)])
    };
    match action {
        SemanticAction::Reduce { position } => {
            let letter = *before
                .word
                .get(position as usize)
                .ok_or("REDUCE inverse position is outside its pre-state")?;
            let sign = if letter > 0 { 1 } else { -1 };
            let positions: Box<dyn Iterator<Item = u32>> = if after.word.is_empty() {
                Box::new(std::iter::once(0))
            } else {
                Box::new(0..after.word.len() as u32)
            };
            for insertion_position in positions {
                let inverse = SemanticAction::Insert {
                    position: insertion_position,
                    generator: letter.unsigned_abs(),
                    sign,
                };
                let inserted = inverse.apply(after)?;
                if let Some(rotation) = rotation_between(&inserted, before) {
                    let mut result = vec![ProofInstruction::Action(inverse)];
                    if rotation != 0 {
                        result.push(ProofInstruction::RotateOriginLeft { amount: rotation });
                    }
                    return Ok(result);
                }
            }
            Err("could not reconstruct the cyclic REDUCE pre-state".into())
        }
        SemanticAction::Commute { position } => exact(SemanticAction::Commute { position }),
        SemanticAction::Braid { position } => exact(SemanticAction::Braid { position }),
        SemanticAction::Insert { position, .. } => exact(SemanticAction::Reduce { position }),
        SemanticAction::Destabilize => {
            let top = before.strands - 1;
            let (position, letter) = before
                .word
                .iter()
                .enumerate()
                .find(|(_, letter)| letter.unsigned_abs() == top)
                .ok_or("DESTABILIZE inverse cannot find the removed top generator")?;
            let inverse = SemanticAction::StabilizeAt {
                position: position.try_into()?,
                sign: if *letter > 0 { 1 } else { -1 },
            };
            exact(inverse)
        }
        SemanticAction::StabilizePositive
        | SemanticAction::StabilizeNegative
        | SemanticAction::StabilizeAt { .. } => exact(SemanticAction::Destabilize),
        SemanticAction::CrossingChange { position } => {
            exact(SemanticAction::CrossingChange { position })
        }
        SemanticAction::Pass => Err("PASS has no proof-program inverse".into()),
        SemanticAction::DescendingCollapse => {
            Err("DESCENDING_COLLAPSE is a one-way theorem macro".into())
        }
        SemanticAction::PlanarCertificateCollapse => {
            Err("PLANAR_CERTIFICATE_COLLAPSE is a one-way theorem macro".into())
        }
    }
}

fn rotation_between(from: &BraidRepresentation, to: &BraidRepresentation) -> Option<u32> {
    if from.strands != to.strands
        || from.cyclic_band_generators != to.cyclic_band_generators
        || from.word.len() != to.word.len()
    {
        return None;
    }
    let len = from.word.len();
    if len == 0 {
        return (from == to).then_some(0);
    }
    (0..len)
        .find(|amount| rotate_word(&from.word, *amount) == to.word)
        .and_then(|amount| amount.try_into().ok())
}

fn coordinate_witness_between(
    from: &BraidRepresentation,
    to: &BraidRepresentation,
) -> Option<NormalizationWitness> {
    for mirrored in [false, true] {
        let base = if mirrored {
            from.mirrored().ok()?
        } else {
            from.clone()
        };
        if let Some(rotate_word_left) = rotation_between(&base, to) {
            return Some(NormalizationWitness {
                mirrored,
                rotate_word_left,
            });
        }
    }
    None
}

fn semantic_first_inverse(
    raw: CheckpointedProofProgram,
    source: &BraidRepresentation,
) -> Result<CheckpointedProofProgram> {
    let first_action_index = raw
        .instructions
        .iter()
        .position(|instruction| matches!(instruction, ProofInstruction::Action(_)))
        .ok_or("inverse edge contains no semantic action")?;
    if first_action_index == 0 {
        return Ok(raw);
    }
    let prefix = &raw.instructions[..first_action_index];
    let transformed_source = replay_instruction_slice(source, prefix)?;
    let source_to_transformed = coordinate_witness_between(source, &transformed_source)
        .ok_or("inverse coordinate prefix is not a mirror/origin transform")?;
    let ProofInstruction::Action(transformed_action) = raw.instructions[first_action_index] else {
        unreachable!();
    };
    let source_len: u32 = source
        .word
        .len()
        .try_into()
        .map_err(|_| "word length does not fit action transport")?;
    let transformed_after = transformed_action.apply(&transformed_source)?;
    let (first_action, after_witness) = match transformed_action {
        SemanticAction::StabilizePositive | SemanticAction::StabilizeNegative => {
            // A global Markov stabilization appends at the current linear
            // origin. Moving an origin prefix past it therefore requires the
            // proof-capable exact-gap form; applying the same global action
            // before the prefix would insert at a different cyclic gap.
            let transformed_sign =
                if matches!(transformed_action, SemanticAction::StabilizePositive) {
                    1
                } else {
                    -1
                };
            let source_sign = if source_to_transformed.mirrored {
                -transformed_sign
            } else {
                transformed_sign
            };
            let mut transported = None;
            for position in 0..=source_len {
                let candidate = SemanticAction::StabilizeAt {
                    position,
                    sign: source_sign,
                };
                let candidate_after = candidate.apply(source)?;
                if let Some(witness) =
                    coordinate_witness_between(&candidate_after, &transformed_after)
                {
                    transported = Some((candidate, witness));
                    break;
                }
            }
            transported.ok_or("cannot transport stabilization through inverse coordinate prefix")?
        }
        _ => {
            let action =
                transformed_action.normalized_to_input(source_to_transformed, source_len)?;
            let after = action.apply(source)?;
            let witness = coordinate_witness_between(&after, &transformed_after)
                .ok_or("cannot transport inverse coordinate prefix through its first action")?;
            (action, witness)
        }
    };

    let mut instructions = vec![ProofInstruction::Action(first_action)];
    if after_witness.mirrored {
        instructions.push(ProofInstruction::MirrorOrbit);
    }
    if after_witness.rotate_word_left != 0 {
        instructions.push(ProofInstruction::RotateOriginLeft {
            amount: after_witness.rotate_word_left,
        });
    }
    instructions.extend_from_slice(&raw.instructions[first_action_index + 1..]);
    Ok(CheckpointedProofProgram { instructions })
}

fn rotate_representation(
    representation: &BraidRepresentation,
    amount: u32,
) -> Result<BraidRepresentation> {
    let len = representation.word.len() as u32;
    if (len == 0 && amount != 0) || (len != 0 && amount >= len) {
        return Err(format!("cannot rotate word length {} left by {}", len, amount).into());
    }
    NormalizationWitness {
        mirrored: false,
        rotate_word_left: amount,
    }
    .apply(representation)
}

pub fn verify_edge_program(
    source_encoding: &[u8],
    target_encoding: &[u8],
    program_encoding: &[u8],
    stated_program_version: u32,
    stated_first_action: u64,
    stated_cc_cost: u8,
) -> Result<()> {
    verify_edge_program_with_certificate(
        source_encoding,
        target_encoding,
        program_encoding,
        stated_program_version,
        stated_first_action,
        stated_cc_cost,
        None,
    )
}

pub fn verify_edge_program_with_certificate(
    source_encoding: &[u8],
    target_encoding: &[u8],
    program_encoding: &[u8],
    stated_program_version: u32,
    stated_first_action: u64,
    stated_cc_cost: u8,
    certificate: Option<&[u8]>,
) -> Result<()> {
    let source = BraidRepresentation::decode(source_encoding)?;
    let target = BraidRepresentation::decode(target_encoding)?;
    if !source.is_normalized()? || !target.is_normalized()? {
        return Err("edge checkpoints must both be normalized".into());
    }
    if program_encoding.len() < PROGRAM_HEADER_LEN
        || program_encoding[..4] != PROGRAM_MAGIC
        || u32::from(program_encoding[4]) != stated_program_version
    {
        return Err("edge program header does not match stated program version".into());
    }
    let (first, cc_cost, replayed, explicit_final_checkpoint) = match stated_program_version {
        0 => {
            let program = ProofProgram::decode(program_encoding)?;
            let first = *program.actions.first().ok_or("proof program is empty")?;
            (first, program.cc_cost(), program.replay(&source)?, false)
        }
        CheckpointedProofProgram::VERSION => {
            let program = CheckpointedProofProgram::decode(program_encoding)?;
            let first = program.first_action()?;
            if program.contains_planar_certificate() {
                let canonical_unknot = BraidRepresentation {
                    strands: 1,
                    cyclic_band_generators: false,
                    word: Vec::new(),
                };
                if target != canonical_unknot {
                    return Err("planar certificate edge target is not the canonical unknot".into());
                }
                let certificate = certificate.ok_or("planar certificate edge has no sidecar")?;
                let mut replayed = source.clone();
                let mut collapsed = false;
                for (index, instruction) in program.instructions.iter().copied().enumerate() {
                    if instruction
                        == ProofInstruction::Action(SemanticAction::PlanarCertificateCollapse)
                    {
                        if collapsed {
                            return Err(
                                "planar certificate program contains multiple collapses".into()
                            );
                        }
                        crate::planar::verify_certificate(&replayed, certificate)?;
                        replayed = canonical_unknot.clone();
                        collapsed = true;
                    } else {
                        if collapsed {
                            return Err(
                                "planar certificate collapse is not the final instruction".into()
                            );
                        }
                        replayed = apply_proof_instruction(&replayed, instruction, index)?;
                    }
                }
                if !collapsed {
                    return Err("planar certificate program has no collapse".into());
                }
                (first, program.cc_cost(), replayed, true)
            } else {
                if certificate.is_some() {
                    return Err("non-planar proof edge carries a planar sidecar".into());
                }
                (first, program.cc_cost(), program.replay(&source)?, true)
            }
        }
        other => return Err(format!("unsupported edge program version {other}").into()),
    };
    if first.encode_u63()? != stated_first_action {
        return Err("edge first_action does not match its program".into());
    }
    if cc_cost != u32::from(stated_cc_cost) {
        return Err(format!(
            "edge states cc_cost={}, program recomputes {}",
            stated_cc_cost, cc_cost
        )
        .into());
    }
    let replayed = if explicit_final_checkpoint {
        if !replayed.is_normalized()? {
            return Err("checkpointed edge program does not end at a normalized origin".into());
        }
        replayed.encode()?
    } else {
        replayed.normalize_origin()?.encoded
    };
    if replayed != target_encoding {
        return Err("normalized replay endpoint does not match target checkpoint".into());
    }
    Ok(())
}

fn transport_position(position: u32, len: u32, shift: u32, inverse: bool) -> Result<u32> {
    if len == 0 {
        return if position == 0 {
            Ok(0)
        } else {
            Err("only position zero exists on an empty word".into())
        };
    }
    if position >= len {
        return Err(format!("position {} is outside word length {}", position, len).into());
    }
    let shift = shift % len;
    Ok(if inverse {
        (position + shift) % len
    } else {
        (position + len - shift) % len
    })
}

fn rotate_word(word: &[i16], shift: usize) -> Vec<i16> {
    if word.is_empty() {
        return Vec::new();
    }
    let shift = shift % word.len();
    let mut rotated = Vec::with_capacity(word.len());
    rotated.extend_from_slice(&word[shift..]);
    rotated.extend_from_slice(&word[..shift]);
    rotated
}

/// Booth's linear-time least-rotation algorithm. `min(i,j)` also gives the
/// smallest origin when a periodic word has several equal minimal rotations.
fn minimal_cyclic_rotation(word: &[i16]) -> usize {
    let n = word.len();
    if n < 2 {
        return 0;
    }
    let (mut i, mut j, mut k) = (0, 1, 0);
    while i < n && j < n && k < n {
        let a = word[(i + k) % n];
        let b = word[(j + k) % n];
        if a == b {
            k += 1;
        } else if a > b {
            i += k + 1;
            if i == j {
                i += 1;
            }
            k = 0;
        } else {
            j += k + 1;
            if i == j {
                j += 1;
            }
            k = 0;
        }
    }
    i.min(j).min(n - 1)
}

fn sha256_key(bytes: &[u8]) -> Result<RepKey> {
    let hex = unknotdb::util::sha256_hex(bytes);
    let mut key = [0_u8; 32];
    for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(pair)?;
        key[index] = u8::from_str_radix(text, 16)?;
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn braid(word: &[i16]) -> BraidRepresentation {
        BraidRepresentation {
            strands: 4,
            cyclic_band_generators: false,
            word: word.to_vec(),
        }
    }

    #[test]
    fn every_word_rotation_has_one_key_and_replayable_origin_witness() {
        let base = braid(&[2, -1, 3, 1]);
        let expected = base.normalize().unwrap();
        for shift in 0..base.word.len() {
            let mut word = base.word[shift..].to_vec();
            word.extend_from_slice(&base.word[..shift]);
            let input = braid(&word);
            let normalized = input.normalize().unwrap();
            assert_eq!(normalized.key, expected.key);
            assert_eq!(normalized.encoded, expected.encoded);
            assert_eq!(
                normalized.witness.apply(&input).unwrap(),
                normalized.representation
            );
        }
    }

    #[test]
    fn mirror_pair_has_one_orbit_key_and_replayable_parity() {
        let positive = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1, 1, 1],
        };
        let negative = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![-1, -1, -1],
        };
        let canonical = positive.normalize().unwrap();
        let reflected = negative.normalize().unwrap();
        assert_eq!(canonical.key, reflected.key);
        assert_eq!(canonical.encoded, reflected.encoded);
        assert!(!canonical.witness.mirrored);
        assert!(reflected.witness.mirrored);
        assert_eq!(
            reflected.witness.apply(&negative).unwrap(),
            positive.normalize().unwrap().representation
        );
        assert_eq!(
            SemanticAction::StabilizeNegative
                .input_to_normalized(reflected.witness, 3)
                .unwrap(),
            SemanticAction::StabilizePositive
        );
    }

    #[test]
    fn knot_closure_parity_is_a_safe_filter_but_not_a_key_quotient() {
        let unknot = BraidRepresentation {
            strands: 1,
            cyclic_band_generators: false,
            word: vec![],
        };
        let two_component_unlink = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![],
        };
        let three_cycle = BraidRepresentation {
            strands: 3,
            cyclic_band_generators: false,
            word: vec![1, 2],
        };
        assert!(unknot.is_knot_closure().unwrap());
        assert!(!two_component_unlink.has_knot_closure_parity());
        assert!(!two_component_unlink.is_knot_closure().unwrap());
        assert!(three_cycle.has_knot_closure_parity());
        assert!(three_cycle.is_knot_closure().unwrap());
        assert_eq!(three_cycle.closure_component_count().unwrap(), 1);
    }

    #[test]
    fn periodic_ties_choose_the_smallest_origin() {
        assert_eq!(minimal_cyclic_rotation(&[1, 2, 1, 2]), 0);
        assert_eq!(minimal_cyclic_rotation(&[2, 1, 2, 1]), 1);
        assert_eq!(minimal_cyclic_rotation(&[1, 1, 1]), 0);
    }

    #[test]
    fn linear_rotation_matches_bruteforce_on_small_words() {
        for length in 0..=7 {
            let cases = 3_usize.pow(length as u32);
            for mut code in 0..cases {
                let mut word = vec![0_i16; length];
                for value in &mut word {
                    *value = [-1, 1, 2][code % 3];
                    code /= 3;
                }
                let expected = (0..length)
                    .min_by_key(|&shift| {
                        (0..length)
                            .map(|index| word[(index + shift) % length])
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or(0);
                assert_eq!(minimal_cyclic_rotation(&word), expected, "{word:?}");
            }
        }
    }

    #[test]
    fn codec_is_exact_and_rejects_padding() {
        let input = BraidRepresentation {
            strands: 4,
            cyclic_band_generators: true,
            word: vec![4, 1, -4],
        };
        let encoded = input.encode().unwrap();
        assert_eq!(BraidRepresentation::decode(&encoded).unwrap(), input);
        let mut bad = input;
        bad.word.push(0);
        assert!(bad
            .encode()
            .unwrap_err()
            .to_string()
            .contains("zero padding"));
    }

    #[test]
    fn packed_storage_round_trips_all_widths_without_changing_keys() {
        for input in [
            BraidRepresentation {
                strands: 4,
                cyclic_band_generators: true,
                word: vec![4, 1, -4],
            },
            BraidRepresentation {
                strands: 20,
                cyclic_band_generators: false,
                word: vec![19, -8, 1],
            },
            BraidRepresentation {
                strands: 300,
                cyclic_band_generators: false,
                word: vec![299, -129, 1],
            },
        ] {
            let canonical = input.encode().unwrap();
            let packed = input.encode_storage().unwrap();
            assert_eq!(BraidRepresentation::decode_storage(&packed).unwrap(), input);
            assert_eq!(
                BraidRepresentation::decode_storage(&canonical).unwrap(),
                input
            );
            assert_eq!(
                BraidRepresentation::decode_storage(&packed)
                    .unwrap()
                    .normalize()
                    .unwrap()
                    .key,
                input.normalize().unwrap().key
            );
        }
        let odd = BraidRepresentation {
            strands: 4,
            cyclic_band_generators: false,
            word: vec![1, -2, 3],
        };
        let mut packed = odd.encode_storage().unwrap();
        assert_eq!(packed.len(), 4 + 2);
        *packed.last_mut().unwrap() |= 0xf0;
        assert!(BraidRepresentation::decode_storage(&packed).is_err());
    }

    #[test]
    fn semantic_action_codec_and_origin_transport_round_trip() {
        let actions = [
            SemanticAction::Reduce { position: 3 },
            SemanticAction::Commute { position: 2 },
            SemanticAction::Braid { position: 1 },
            SemanticAction::Insert {
                position: 4,
                generator: 3,
                sign: -1,
            },
            SemanticAction::Destabilize,
            SemanticAction::StabilizePositive,
            SemanticAction::StabilizeNegative,
            SemanticAction::StabilizeAt {
                position: 4,
                sign: -1,
            },
            SemanticAction::Pass,
            SemanticAction::CrossingChange { position: 0 },
            SemanticAction::PlanarCertificateCollapse,
        ];
        let witness = NormalizationWitness {
            mirrored: false,
            rotate_word_left: 2,
        };
        for action in actions {
            let encoded = action.encode_u63().unwrap();
            assert_eq!(SemanticAction::decode_u63(encoded).unwrap(), action);
            let normalized = action.input_to_normalized(witness, 5).unwrap();
            assert_eq!(normalized.normalized_to_input(witness, 5).unwrap(), action);
        }
        assert_eq!(
            SemanticAction::CrossingChange { position: 0 }
                .input_to_normalized(witness, 5)
                .unwrap(),
            SemanticAction::CrossingChange { position: 3 }
        );
    }

    #[test]
    fn anchored_templates_factor_a_shared_torus_translation() {
        let source = BraidRepresentation {
            strands: 3,
            cyclic_band_generators: false,
            word: vec![1, 2, 1, -2],
        };
        let translated = |anchor: u32, crossing: u32| CheckpointedProofProgram {
            instructions: vec![
                ProofInstruction::Action(SemanticAction::Insert {
                    position: anchor,
                    generator: 1,
                    sign: 1,
                }),
                ProofInstruction::Action(SemanticAction::Reduce { position: anchor }),
                ProofInstruction::Action(SemanticAction::CrossingChange { position: crossing }),
            ],
        };
        let at_three = translated(3, 1);
        let at_one = translated(1, 3);
        let anchored_three = at_three.anchor_template(&source).unwrap();
        let anchored_one = at_one.anchor_template(&source).unwrap();

        assert_eq!(anchored_three.anchor_y, 3);
        assert_eq!(anchored_one.anchor_y, 1);
        assert_eq!(anchored_three.anchor_x, 1);
        assert_eq!(anchored_three.template, anchored_one.template);
        assert_eq!(anchored_three.materialize(&source).unwrap(), at_three);
        assert_eq!(anchored_one.materialize(&source).unwrap(), at_one);
        let mut wrong_binding = anchored_one;
        wrong_binding.anchor_x = 2;
        assert!(wrong_binding.materialize(&source).is_err());
    }

    #[test]
    fn proof_program_replays_crossing_change_macro_to_normalized_target() {
        let source = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1, 1, 1],
        }
        .normalize()
        .unwrap();
        let target = BraidRepresentation {
            strands: 1,
            cyclic_band_generators: false,
            word: vec![],
        }
        .normalize()
        .unwrap();
        let program = ProofProgram {
            actions: vec![
                SemanticAction::CrossingChange { position: 0 },
                SemanticAction::Reduce { position: 0 },
                SemanticAction::Destabilize,
            ],
        };
        let encoded = program.encode().unwrap();
        assert_eq!(ProofProgram::decode(&encoded).unwrap(), program);
        verify_edge_program(
            &source.encoded,
            &target.encoded,
            &encoded,
            0,
            program.actions[0].encode_u63().unwrap(),
            1,
        )
        .unwrap();
    }

    #[test]
    fn planar_certificate_accepts_an_exact_zero_cost_prefix() {
        let source = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![-1, 1, 1],
        };
        let target = BraidRepresentation {
            strands: 1,
            cyclic_band_generators: false,
            word: vec![],
        };
        let program = CheckpointedProofProgram {
            instructions: vec![
                ProofInstruction::MirrorOrbit,
                ProofInstruction::MirrorOrbit,
                ProofInstruction::Action(SemanticAction::PlanarCertificateCollapse),
            ],
        };
        verify_edge_program_with_certificate(
            &source.encode().unwrap(),
            &target.encode().unwrap(),
            &program.encode().unwrap(),
            CheckpointedProofProgram::VERSION,
            SemanticAction::PlanarCertificateCollapse
                .encode_u63()
                .unwrap(),
            0,
            Some(crate::planar::TREFOIL_TEST_CERTIFICATE),
        )
        .unwrap();
    }

    #[test]
    fn state_aware_inverse_covers_every_semantic_primitive() {
        let cases = vec![
            (
                BraidRepresentation {
                    strands: 3,
                    cyclic_band_generators: false,
                    word: vec![1, -1, 2],
                },
                SemanticAction::Reduce { position: 0 },
            ),
            (
                BraidRepresentation {
                    strands: 3,
                    cyclic_band_generators: false,
                    word: vec![-1, 2, 1],
                },
                SemanticAction::Reduce { position: 2 },
            ),
            (
                BraidRepresentation {
                    strands: 4,
                    cyclic_band_generators: false,
                    word: vec![1, 3],
                },
                SemanticAction::Commute { position: 0 },
            ),
            (
                BraidRepresentation {
                    strands: 3,
                    cyclic_band_generators: false,
                    word: vec![1, 2, 1],
                },
                SemanticAction::Braid { position: 0 },
            ),
            (
                BraidRepresentation {
                    strands: 3,
                    cyclic_band_generators: false,
                    word: vec![2],
                },
                SemanticAction::Insert {
                    position: 0,
                    generator: 1,
                    sign: 1,
                },
            ),
            (
                BraidRepresentation {
                    strands: 3,
                    cyclic_band_generators: false,
                    word: vec![1, 2, 1],
                },
                SemanticAction::Destabilize,
            ),
            (
                BraidRepresentation {
                    strands: 2,
                    cyclic_band_generators: false,
                    word: vec![1],
                },
                SemanticAction::StabilizeNegative,
            ),
            (
                BraidRepresentation {
                    strands: 3,
                    cyclic_band_generators: false,
                    word: vec![1, -2, 1],
                },
                SemanticAction::StabilizeAt {
                    position: 1,
                    sign: 1,
                },
            ),
            (
                BraidRepresentation {
                    strands: 2,
                    cyclic_band_generators: false,
                    word: vec![1, 1, 1],
                },
                SemanticAction::CrossingChange { position: 1 },
            ),
        ];

        for (source, action) in cases {
            let forward = CheckpointedProofProgram {
                instructions: vec![ProofInstruction::Action(action)],
            };
            let target = forward.replay(&source).unwrap();
            let inverse = forward.invert_edge(&source).unwrap();
            assert_eq!(inverse.source, target);
            assert_eq!(inverse.target, source);
            assert_eq!(inverse.program.replay(&inverse.source).unwrap(), source);
            assert_eq!(inverse.program.cc_cost(), forward.cc_cost());
            inverse.program.first_action().unwrap();
        }
    }

    #[test]
    fn rank10_frontier_destabilize_inverse_preserves_middle_word_gap() {
        let source = BraidRepresentation {
            strands: 3,
            cyclic_band_generators: false,
            word: vec![-2, -1, 2, -1, 2, -1, 2, -1, 2, -1],
        };
        let forward = CheckpointedProofProgram {
            instructions: vec![
                ProofInstruction::Action(SemanticAction::Insert {
                    position: 0,
                    generator: 1,
                    sign: 1,
                }),
                ProofInstruction::Action(SemanticAction::StabilizePositive),
                ProofInstruction::Action(SemanticAction::CrossingChange { position: 0 }),
                ProofInstruction::NormalizeOrigin {
                    rotate_word_left: 2,
                },
                ProofInstruction::Action(SemanticAction::Destabilize),
                ProofInstruction::MirrorOrbit,
                ProofInstruction::NormalizeOrigin {
                    rotate_word_left: 2,
                },
            ],
        };
        let target = forward.replay(&source).unwrap();
        let inverse = forward.invert_edge(&source).unwrap();
        assert_eq!(inverse.source, target);
        assert_eq!(inverse.target, source);
        assert_eq!(inverse.program.replay(&target).unwrap(), source);
        assert!(inverse.program.instructions.iter().any(|instruction| {
            matches!(
                instruction,
                ProofInstruction::Action(SemanticAction::StabilizeAt { .. })
            )
        }));
    }

    #[test]
    fn inverse_conjugates_leading_origin_and_mirror_changes_past_first_action() {
        let source = BraidRepresentation {
            strands: 4,
            cyclic_band_generators: false,
            word: vec![1, 3],
        };
        let forward = CheckpointedProofProgram {
            instructions: vec![
                ProofInstruction::Action(SemanticAction::Commute { position: 0 }),
                ProofInstruction::NormalizeOrigin {
                    rotate_word_left: 1,
                },
                ProofInstruction::MirrorOrbit,
            ],
        };
        let target = forward.replay(&source).unwrap();
        let inverse = forward.invert_edge(&source).unwrap();
        assert_eq!(inverse.source, target);
        assert!(matches!(
            inverse.program.instructions.first(),
            Some(ProofInstruction::Action(_))
        ));
        assert_eq!(inverse.program.replay(&target).unwrap(), source);
        assert!(inverse
            .program
            .instructions
            .iter()
            .any(|instruction| matches!(instruction, ProofInstruction::MirrorOrbit)));
    }

    #[test]
    fn checkpointed_program_replays_and_validates_intermediate_origin_change() {
        let source = BraidRepresentation {
            strands: 3,
            cyclic_band_generators: false,
            word: vec![-2, 1, 2],
        }
        .normalize()
        .unwrap();
        let target = BraidRepresentation {
            strands: 3,
            cyclic_band_generators: false,
            word: vec![1, 2, 2],
        }
        .normalize()
        .unwrap();
        let first = SemanticAction::CrossingChange { position: 0 };
        let program = CheckpointedProofProgram {
            instructions: vec![
                ProofInstruction::Action(first),
                ProofInstruction::NormalizeOrigin {
                    rotate_word_left: 1,
                },
            ],
        };
        let encoded = program.encode().unwrap();
        assert_eq!(CheckpointedProofProgram::decode(&encoded).unwrap(), program);
        verify_edge_program(
            &source.encoded,
            &target.encoded,
            &encoded,
            CheckpointedProofProgram::VERSION,
            first.encode_u63().unwrap(),
            1,
        )
        .unwrap();

        let wrong_witness = CheckpointedProofProgram {
            instructions: vec![
                ProofInstruction::Action(first),
                ProofInstruction::NormalizeOrigin {
                    rotate_word_left: 2,
                },
            ],
        };
        assert!(wrong_witness
            .replay(&source.representation)
            .unwrap_err()
            .to_string()
            .contains("recomputed 1"));
    }

    #[test]
    fn coordinate_only_program_has_an_exact_non_edge_inverse() {
        let source = braid(&[2, 1]);
        let program = CheckpointedProofProgram {
            instructions: vec![ProofInstruction::NormalizeOrigin {
                rotate_word_left: 1,
            }],
        };
        let target = program.replay(&source).unwrap();
        let inverse = program.invert_exact(&source).unwrap();
        assert_eq!(inverse.source, target);
        assert_eq!(inverse.target, source);
        assert_eq!(inverse.program.replay(&inverse.source).unwrap(), source);
        assert!(program.invert_edge(&source).is_err());

        let prefixed = CheckpointedProofProgram {
            instructions: vec![
                ProofInstruction::NormalizeOrigin {
                    rotate_word_left: 1,
                },
                ProofInstruction::Action(SemanticAction::CrossingChange { position: 0 }),
            ],
        };
        let expected = prefixed.replay(&source).unwrap();
        let semantic_first = prefixed.canonicalize_semantic_first(&source).unwrap();
        assert!(semantic_first.first_action().unwrap().is_crossing_change());
        assert_eq!(semantic_first.replay(&source).unwrap(), expected);
    }

    #[test]
    fn semantic_first_transports_stabilization_to_an_exact_word_gap() {
        let source = braid(&[1, -2, 1, 2]);
        let prefixed = CheckpointedProofProgram {
            instructions: vec![
                ProofInstruction::RotateOriginLeft { amount: 2 },
                ProofInstruction::Action(SemanticAction::StabilizePositive),
            ],
        };
        let expected = prefixed.replay(&source).unwrap();
        let semantic_first = prefixed.canonicalize_semantic_first(&source).unwrap();
        assert!(matches!(
            semantic_first.first_action().unwrap(),
            SemanticAction::StabilizeAt { sign: 1, .. }
        ));
        assert_eq!(semantic_first.replay(&source).unwrap(), expected);
    }

    #[test]
    fn proof_replay_rejects_illegal_and_control_actions() {
        let source = braid(&[1, 2]);
        assert!(SemanticAction::Reduce { position: 0 }
            .apply(&source)
            .unwrap_err()
            .to_string()
            .contains("does not cancel"));
        assert!(SemanticAction::Pass
            .apply(&source)
            .unwrap_err()
            .to_string()
            .contains("controller action"));
    }

    #[test]
    fn descending_v0_changes_one_crossing_of_the_positive_trefoil() {
        let trefoil = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1, 1, 1],
        };
        let changes = descending_crossing_changes_v0(&trefoil).unwrap();
        assert_eq!(changes, vec![1]);
        let descending = SemanticAction::CrossingChange {
            position: changes[0],
        }
        .apply(&trefoil)
        .unwrap();
        assert!(descending_crossing_changes_v0(&descending)
            .unwrap()
            .is_empty());
        let collapsed = SemanticAction::DescendingCollapse
            .apply(&descending)
            .unwrap();
        assert_eq!(collapsed.strands, 1);
        assert!(collapsed.word.is_empty());
        let ascending = descending.mirrored().unwrap();
        assert_eq!(
            descending_crossing_changes_v0(&ascending).unwrap().len(),
            ascending.word.len()
        );
        assert!(SemanticAction::DescendingCollapse.apply(&ascending).is_ok());
        assert!(SemanticAction::DescendingCollapse.apply(&trefoil).is_err());
    }
}
