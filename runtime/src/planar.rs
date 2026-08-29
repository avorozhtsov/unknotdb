//! Independent replay of labelled planar RI/RII/RIII certificates.

use crate::representation::BraidRepresentation;
use crate::{RepKey, Result};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

pub const CERTIFICATE_FORMAT: &str = "unknotdb-labelled-reidemeister-trace-v0";
pub const CERTIFICATE_FORMAT_V1: &str = "unknotdb-labelled-reidemeister-trace-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Endpoint {
    crossing: usize,
    index: usize,
}

#[derive(Clone, Debug)]
struct Crossing {
    sign: i8,
    adjacent: [Endpoint; 4],
    alive: bool,
}

#[derive(Clone, Debug)]
struct Diagram {
    crossings: Vec<Crossing>,
}

struct Certificate {
    format: String,
    input: CertificateInput,
    moves: Vec<CertificateMove>,
    final_: Option<CertificateFinal>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CertificateFinal {
    crossings: usize,
    state_sha256: String,
    unlinked_unknot_components: usize,
}

#[derive(Deserialize)]
struct CertificateInput {
    ordinary_artin_braid_word: Vec<i16>,
    crossings: usize,
}

struct CertificateMove {
    move_: String,
    at: Option<String>,
    eliminated: Option<Vec<String>>,
    triple: Option<Vec<(String, usize)>>,
    added: Option<serde_json::Value>,
    signs: Option<HashMap<String, i8>>,
    state: Option<Vec<StateCrossing>>,
    before_sha256: String,
    after_sha256: String,
    remaining_crossings: usize,
}

#[derive(Clone, Deserialize)]
struct StateCrossing {
    label: String,
    sign: i8,
    adjacent: Vec<(String, usize)>,
}

impl<'de> Deserialize<'de> for CertificateMove {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            #[serde(rename = "move")]
            move_: String,
            at: Option<String>,
            eliminated: Option<Vec<String>>,
            triple: Option<Vec<(String, usize)>>,
            added: Option<serde_json::Value>,
            signs: Option<HashMap<String, i8>>,
            state: Option<Vec<StateCrossing>>,
            before_sha256: String,
            after_sha256: String,
            remaining_crossings: usize,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            move_: raw.move_,
            at: raw.at,
            eliminated: raw.eliminated,
            triple: raw.triple,
            added: raw.added,
            signs: raw.signs,
            state: raw.state,
            before_sha256: raw.before_sha256,
            after_sha256: raw.after_sha256,
            remaining_crossings: raw.remaining_crossings,
        })
    }
}

impl<'de> Deserialize<'de> for Certificate {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            format: String,
            input: CertificateInput,
            moves: Vec<CertificateMove>,
            #[serde(rename = "final")]
            final_: Option<CertificateFinal>,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(Self {
            format: raw.format,
            input: raw.input,
            moves: raw.moves,
            final_: raw.final_,
        })
    }
}

impl Diagram {
    fn from_state(state: &[StateCrossing]) -> Result<Self> {
        let parse = |label: &str| -> Result<usize> {
            Ok(label
                .strip_prefix('x')
                .ok_or("planar state label does not start with x")?
                .parse()?)
        };
        let max_index = state
            .iter()
            .map(|crossing| parse(&crossing.label))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .max()
            .ok_or("planar expanded state is empty")?;
        let mut crossings = (0..=max_index)
            .map(|index| Crossing {
                sign: 0,
                adjacent: [Endpoint {
                    crossing: index,
                    index: 0,
                }; 4],
                alive: false,
            })
            .collect::<Vec<_>>();
        for item in state {
            let index = parse(&item.label)?;
            if item.adjacent.len() != 4 || crossings[index].alive {
                return Err("planar expanded state has malformed/duplicate crossing".into());
            }
            crossings[index].alive = true;
            crossings[index].sign = item.sign;
            for (slot, (label, endpoint_index)) in item.adjacent.iter().enumerate() {
                if *endpoint_index >= 4 {
                    return Err("planar expanded endpoint index is outside 0..4".into());
                }
                crossings[index].adjacent[slot] = Endpoint {
                    crossing: parse(label)?,
                    index: *endpoint_index,
                };
            }
        }
        for (crossing_index, crossing) in crossings.iter().enumerate() {
            if !crossing.alive {
                continue;
            }
            for (index, endpoint) in crossing.adjacent.iter().enumerate() {
                let peer = crossings
                    .get(endpoint.crossing)
                    .ok_or("planar expanded endpoint crossing is absent")?;
                if !peer.alive
                    || peer.adjacent[endpoint.index]
                        != (Endpoint {
                            crossing: crossing_index,
                            index,
                        })
                {
                    return Err("planar expanded state adjacency is not reciprocal".into());
                }
            }
        }
        Ok(Self { crossings })
    }

    fn from_braid(input: &BraidRepresentation) -> Result<Self> {
        input.validate()?;
        if input.cyclic_band_generators {
            return Err("planar certificate supports ordinary Artin braids only".into());
        }
        let mut crossings = Vec::with_capacity(input.word.len());
        let mut first = vec![None; usize::from(input.strands)];
        let mut pending = vec![None; usize::from(input.strands)];
        for (crossing, &letter) in input.word.iter().enumerate() {
            let left = usize::from(letter.unsigned_abs() - 1);
            let right = left + 1;
            // Spherogram's local endpoint numbering is sign-dependent.  These
            // are its exact top-left, top-right, bottom-left, bottom-right
            // indices for an ordinary Artin braid crossing.
            let (top_left, top_right, bottom_left, bottom_right) = if letter > 0 {
                (0, 3, 1, 2)
            } else {
                (1, 0, 2, 3)
            };
            crossings.push(Crossing {
                sign: if letter > 0 { 1 } else { -1 },
                adjacent: [
                    Endpoint { crossing, index: 0 },
                    Endpoint { crossing, index: 1 },
                    Endpoint { crossing, index: 2 },
                    Endpoint { crossing, index: 3 },
                ],
                alive: true,
            });
            for (position, top) in [(left, top_left), (right, top_right)] {
                let endpoint = Endpoint {
                    crossing,
                    index: top,
                };
                if let Some(previous) = pending[position] {
                    Self::connect_raw(&mut crossings, previous, endpoint);
                } else {
                    first[position] = Some(endpoint);
                }
            }
            pending[left] = Some(Endpoint {
                crossing,
                index: bottom_left,
            });
            pending[right] = Some(Endpoint {
                crossing,
                index: bottom_right,
            });
        }
        for position in 0..pending.len() {
            match (pending[position], first[position]) {
                (Some(bottom), Some(top)) => Self::connect_raw(&mut crossings, bottom, top),
                (None, None) => {}
                _ => return Err("braid closure has an unmatched vertical endpoint".into()),
            }
        }
        Ok(Self { crossings })
    }

    fn connect_raw(crossings: &mut [Crossing], a: Endpoint, b: Endpoint) {
        crossings[a.crossing].adjacent[a.index] = b;
        crossings[b.crossing].adjacent[b.index] = a;
    }

    fn connect(&mut self, a: Endpoint, b: Endpoint) {
        Self::connect_raw(&mut self.crossings, a, b);
    }

    fn alive_count(&self) -> usize {
        self.crossings
            .iter()
            .filter(|crossing| crossing.alive)
            .count()
    }

    fn is_endpoint_reindexing_of(&self, source: &Self) -> bool {
        if self.crossings.len() != source.crossings.len() {
            return false;
        }
        let mut candidates = vec![Vec::<usize>::new(); source.crossings.len()];
        for (index, options) in candidates.iter_mut().enumerate() {
            let old = &source.crossings[index];
            let new = &self.crossings[index];
            if old.alive != new.alive {
                return false;
            }
            if !old.alive {
                continue;
            }
            for rotation in 0..4 {
                if (0..4).all(|slot| {
                    new.adjacent[(slot + rotation) % 4].crossing == old.adjacent[slot].crossing
                }) {
                    options.push(rotation);
                }
            }
            if options.is_empty() {
                return false;
            }
        }

        fn search(
            source: &Diagram,
            target: &Diagram,
            candidates: &[Vec<usize>],
            rotations: &mut [Option<usize>],
        ) -> bool {
            let Some(start) = source
                .crossings
                .iter()
                .enumerate()
                .filter(|(index, crossing)| crossing.alive && rotations[*index].is_none())
                .min_by_key(|(index, _)| candidates[*index].len())
                .map(|(index, _)| index)
            else {
                return true;
            };
            for &initial in &candidates[start] {
                let mut pending = vec![(start, initial)];
                let mut added = Vec::new();
                let mut valid = true;
                while let Some((crossing, rotation)) = pending.pop() {
                    if let Some(existing) = rotations[crossing] {
                        if existing != rotation {
                            valid = false;
                            break;
                        }
                        continue;
                    }
                    if !candidates[crossing].contains(&rotation) {
                        valid = false;
                        break;
                    }
                    rotations[crossing] = Some(rotation);
                    added.push(crossing);
                    for slot in 0..4 {
                        let old_peer = source.crossings[crossing].adjacent[slot];
                        let new_peer = target.crossings[crossing].adjacent[(slot + rotation) % 4];
                        if old_peer.crossing != new_peer.crossing {
                            valid = false;
                            break;
                        }
                        pending
                            .push((old_peer.crossing, (new_peer.index + 4 - old_peer.index) % 4));
                    }
                    if !valid {
                        break;
                    }
                }
                if valid && search(source, target, candidates, rotations) {
                    return true;
                }
                for crossing in added {
                    rotations[crossing] = None;
                }
            }
            false
        }

        search(
            source,
            self,
            &candidates,
            &mut vec![None; source.crossings.len()],
        )
    }

    fn parse_label(&self, label: &str) -> Result<usize> {
        let index: usize = label
            .strip_prefix('x')
            .ok_or("crossing label does not start with x")?
            .parse()?;
        if !self
            .crossings
            .get(index)
            .is_some_and(|crossing| crossing.alive)
        {
            return Err(format!("crossing label {label} is absent").into());
        }
        Ok(index)
    }

    fn label(index: usize) -> String {
        format!("x{index}")
    }

    fn state_json(&self) -> String {
        let mut indices: Vec<_> = self
            .crossings
            .iter()
            .enumerate()
            .filter_map(|(index, crossing)| crossing.alive.then_some(index))
            .collect();
        indices.sort_by_key(|index| Self::label(*index));
        let mut json = String::from("[");
        for (offset, &index) in indices.iter().enumerate() {
            if offset != 0 {
                json.push(',');
            }
            let crossing = &self.crossings[index];
            json.push_str("{\"adjacent\":[");
            for (endpoint_index, endpoint) in crossing.adjacent.iter().enumerate() {
                if endpoint_index != 0 {
                    json.push(',');
                }
                json.push_str(&format!(
                    "[\"{}\",{}]",
                    Self::label(endpoint.crossing),
                    endpoint.index
                ));
            }
            json.push_str(&format!(
                "],\"label\":\"{}\",\"sign\":{}}}",
                Self::label(index),
                crossing.sign
            ));
        }
        json.push(']');
        json
    }

    fn state_sha256(&self) -> RepKey {
        unknotdb::util::sha256(self.state_json().as_bytes())
    }

    fn reidemeister_i(&mut self, crossing: usize) -> Vec<usize> {
        let adjacent = self.crossings[crossing].adjacent;
        for index in 0..4 {
            if adjacent[index]
                == (Endpoint {
                    crossing,
                    index: (index + 1) % 4,
                })
            {
                let a = adjacent[(index + 2) % 4];
                let b = adjacent[(index + 3) % 4];
                if a.crossing != crossing {
                    self.connect(a, b);
                }
                self.crossings[crossing].alive = false;
                return vec![crossing];
            }
        }
        Vec::new()
    }

    fn reidemeister_i_or_ii(&mut self, a_crossing: usize) -> Vec<usize> {
        let eliminated = self.reidemeister_i(a_crossing);
        if !eliminated.is_empty() {
            return eliminated;
        }
        for a in 0..4 {
            let adjacent = self.crossings[a_crossing].adjacent;
            let b_endpoint = adjacent[a];
            let c_endpoint = adjacent[(a + 1) % 4];
            if b_endpoint.crossing == c_endpoint.crossing
                && (b_endpoint.index + 3) % 4 == c_endpoint.index
                && (a + b_endpoint.index).is_multiple_of(2)
            {
                let b_crossing = b_endpoint.crossing;
                let nested = self.reidemeister_i(b_crossing);
                if !nested.is_empty() {
                    return nested;
                }
                let adjacent_a = self.crossings[a_crossing].adjacent;
                let adjacent_b = self.crossings[b_crossing].adjacent;
                let w = adjacent_a[(a + 2) % 4];
                let x = adjacent_a[(a + 3) % 4];
                let y = adjacent_b[(b_endpoint.index + 1) % 4];
                let z = adjacent_b[(b_endpoint.index + 2) % 4];
                if w.crossing != b_crossing {
                    self.connect(w, z);
                }
                if x.crossing != b_crossing {
                    self.connect(x, y);
                }
                self.crossings[a_crossing].alive = false;
                self.crossings[b_crossing].alive = false;
                return vec![a_crossing, b_crossing];
            }
        }
        Vec::new()
    }

    fn next_corner(&self, endpoint: Endpoint) -> Endpoint {
        self.crossings[endpoint.crossing].adjacent[(endpoint.index + 1) % 4]
    }

    fn possible_riii(&self) -> Result<Vec<[Endpoint; 3]>> {
        let mut unused = HashSet::new();
        for (crossing, value) in self.crossings.iter().enumerate() {
            if value.alive {
                for index in 0..4 {
                    unused.insert(Endpoint { crossing, index });
                }
            }
        }
        let mut faces = Vec::new();
        while let Some(&start) = unused.iter().next() {
            let mut face = vec![start];
            unused.remove(&start);
            loop {
                let next = self.next_corner(*face.last().unwrap());
                if next == start {
                    break;
                }
                if !unused.remove(&next) || face.len() > self.alive_count() * 4 {
                    return Err("planar diagram face traversal is malformed".into());
                }
                face.push(next);
            }
            faces.push(face);
        }
        let mut result = Vec::new();
        for mut face in faces {
            if face.len() != 3 {
                continue;
            }
            let odd = face
                .iter()
                .filter(|endpoint| endpoint.index % 2 == 1)
                .count();
            if odd != 1 && odd != 2 {
                continue;
            }
            for _ in 0..3 {
                if face[1].index % 2 == 0 && face[2].index % 2 == 1 {
                    break;
                }
                face.rotate_left(1);
            }
            if face[1].index % 2 != 0 || face[2].index % 2 != 1 {
                return Err("RIII face could not be put in canonical order".into());
            }
            if face
                .iter()
                .map(|endpoint| endpoint.crossing)
                .collect::<HashSet<_>>()
                .len()
                == 3
            {
                result.push([face[0], face[1], face[2]]);
            }
        }
        Ok(result)
    }

    fn reidemeister_iii(&mut self, triple: [Endpoint; 3]) -> Result<()> {
        if !self.possible_riii()?.contains(&triple) {
            return Err("recorded RIII triple is not an available type-III move".into());
        }
        let [a, b, c] = triple;
        let old_border = [
            Endpoint {
                crossing: c.crossing,
                index: (c.index + 3) % 4,
            },
            Endpoint {
                crossing: c.crossing,
                index: (c.index + 2) % 4,
            },
            Endpoint {
                crossing: a.crossing,
                index: (a.index + 3) % 4,
            },
            Endpoint {
                crossing: a.crossing,
                index: (a.index + 2) % 4,
            },
            Endpoint {
                crossing: b.crossing,
                index: (b.index + 3) % 4,
            },
            Endpoint {
                crossing: b.crossing,
                index: (b.index + 2) % 4,
            },
        ];
        let new_border = [
            a,
            Endpoint {
                crossing: b.crossing,
                index: (b.index + 1) % 4,
            },
            b,
            Endpoint {
                crossing: c.crossing,
                index: (c.index + 1) % 4,
            },
            c,
            Endpoint {
                crossing: a.crossing,
                index: (a.index + 1) % 4,
            },
        ];
        // Replay Spherogram's operation literally, including its six temporary
        // two-ended Strand objects.  A direct border shortcut is not equivalent
        // for degenerate local adjacencies because endpoint assignments leave
        // the displaced peer untouched until the temporary strand is fused.
        let crossing_endpoint_count = self.crossings.len() * 4;
        let endpoint_id = |endpoint: Endpoint| endpoint.crossing * 4 + endpoint.index;
        let mut adjacent = vec![0_usize; crossing_endpoint_count + 12];
        for (crossing_index, crossing) in self.crossings.iter().enumerate() {
            for (index, endpoint) in crossing.adjacent.iter().copied().enumerate() {
                adjacent[crossing_index * 4 + index] = endpoint_id(endpoint);
            }
        }
        let connect = |adjacent: &mut [usize], left: usize, right: usize| {
            adjacent[left] = right;
            adjacent[right] = left;
        };
        for (index, endpoint) in old_border.iter().copied().enumerate() {
            let old = endpoint_id(endpoint);
            let external = adjacent[old];
            let strand_0 = crossing_endpoint_count + 2 * index;
            let strand_1 = strand_0 + 1;
            connect(&mut adjacent, strand_0, old);
            connect(&mut adjacent, strand_1, external);
        }
        for (index, endpoint) in new_border.iter().copied().enumerate() {
            connect(
                &mut adjacent,
                endpoint_id(endpoint),
                crossing_endpoint_count + 2 * index,
            );
        }
        connect(
            &mut adjacent,
            endpoint_id(old_border[2]),
            endpoint_id(Endpoint {
                crossing: b.crossing,
                index: (b.index + 2) % 4,
            }),
        );
        connect(
            &mut adjacent,
            endpoint_id(old_border[4]),
            endpoint_id(Endpoint {
                crossing: c.crossing,
                index: (c.index + 2) % 4,
            }),
        );
        connect(
            &mut adjacent,
            endpoint_id(old_border[0]),
            endpoint_id(Endpoint {
                crossing: a.crossing,
                index: (a.index + 2) % 4,
            }),
        );
        for index in 0..6 {
            let strand_0 = crossing_endpoint_count + 2 * index;
            let strand_1 = strand_0 + 1;
            let left = adjacent[strand_0];
            let right = adjacent[strand_1];
            connect(&mut adjacent, left, right);
        }
        for crossing_index in 0..self.crossings.len() {
            for index in 0..4 {
                let peer = adjacent[crossing_index * 4 + index];
                if peer >= crossing_endpoint_count {
                    return Err("RIII replay left an unfused temporary strand".into());
                }
                self.crossings[crossing_index].adjacent[index] = Endpoint {
                    crossing: peer / 4,
                    index: peer % 4,
                };
            }
        }
        Ok(())
    }
}

fn parse_sha256(value: &str) -> Result<RepKey> {
    if value.len() != 64 {
        return Err("planar checkpoint SHA-256 must have 64 hex digits".into());
    }
    let mut result = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks(2).enumerate() {
        result[index] = u8::from_str_radix(std::str::from_utf8(pair)?, 16)?;
    }
    Ok(result)
}

pub fn verify_certificate(source: &BraidRepresentation, bytes: &[u8]) -> Result<()> {
    let certificate: Certificate = serde_json::from_slice(bytes)?;
    if certificate.format != CERTIFICATE_FORMAT && certificate.format != CERTIFICATE_FORMAT_V1 {
        return Err(format!("unsupported planar certificate `{}`", certificate.format).into());
    }
    let final_checkpoint = certificate
        .final_
        .as_ref()
        .ok_or("planar certificate has no final checkpoint")?;
    if certificate.input.crossings != certificate.input.ordinary_artin_braid_word.len() {
        return Err("planar certificate input crossing count is inconsistent".into());
    }
    if certificate.input.ordinary_artin_braid_word != source.word {
        return Err("planar certificate input word does not match edge source".into());
    }
    let mut diagram = Diagram::from_braid(source)?;
    for (move_index, step) in certificate.moves.iter().enumerate() {
        let actual_before = diagram.state_sha256();
        let expected_before = parse_sha256(&step.before_sha256)?;
        if actual_before != expected_before {
            return Err(format!(
                "planar move {move_index} input checkpoint mismatch: expected {}, got {}",
                step.before_sha256,
                sha256_hex(actual_before)
            )
            .into());
        }
        match step.move_.as_str() {
            "RI" | "RII" => {
                let at = diagram.parse_label(step.at.as_deref().ok_or("RI/RII has no anchor")?)?;
                let mut eliminated = diagram.reidemeister_i_or_ii(at);
                eliminated.sort_by_key(|index| Diagram::label(*index));
                let labels: Vec<_> = eliminated.into_iter().map(Diagram::label).collect();
                if labels
                    != *step
                        .eliminated
                        .as_ref()
                        .ok_or("RI/RII has no eliminated set")?
                    || labels.len() != if step.move_ == "RI" { 1 } else { 2 }
                {
                    return Err(format!("planar move {move_index} is not the stated move").into());
                }
            }
            "RIII" => {
                let raw = step.triple.as_ref().ok_or("RIII has no triple")?;
                if raw.len() != 3 {
                    return Err("RIII triple does not have three endpoints".into());
                }
                let mut triple = [Endpoint {
                    crossing: 0,
                    index: 0,
                }; 3];
                for (slot, (label, index)) in raw.iter().enumerate() {
                    if *index >= 4 {
                        return Err("RIII endpoint index is outside 0..4".into());
                    }
                    triple[slot] = Endpoint {
                        crossing: diagram.parse_label(label)?,
                        index: *index,
                    };
                }
                diagram.reidemeister_iii(triple)?;
            }
            "RI+" | "RII+" => {
                if certificate.format != CERTIFICATE_FORMAT_V1 {
                    return Err("inverse planar moves require certificate format v1".into());
                }
                let expected_added = if step.move_ == "RI+" { 1 } else { 2 };
                let raw_added = step
                    .added
                    .as_ref()
                    .ok_or("inverse move has no added labels")?;
                let added_labels: Vec<String> = if expected_added == 1 {
                    vec![raw_added
                        .as_str()
                        .ok_or("RI+ added label is not a string")?
                        .to_owned()]
                } else {
                    raw_added
                        .as_array()
                        .ok_or("RII+ added labels are not an array")?
                        .iter()
                        .map(|value| {
                            value
                                .as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| "RII+ added label is not a string".into())
                        })
                        .collect::<Result<_>>()?
                };
                if added_labels.len() != expected_added {
                    return Err("inverse move added-label count is inconsistent".into());
                }
                let mut target = Diagram::from_state(
                    step.state
                        .as_deref()
                        .ok_or("inverse move has no expanded state")?,
                )?;
                while target.crossings.len() < diagram.crossings.len() {
                    let index = target.crossings.len();
                    target.crossings.push(Crossing {
                        sign: 0,
                        adjacent: [Endpoint {
                            crossing: index,
                            index: 0,
                        }; 4],
                        alive: false,
                    });
                }
                if target.alive_count() != diagram.alive_count() + expected_added {
                    return Err("inverse move expanded-state crossing count is wrong".into());
                }
                let expected_before = diagram.state_sha256();
                let mut accepted = false;
                for label in &added_labels {
                    let mut reduced = target.clone();
                    let anchor = reduced.parse_label(label)?;
                    let mut eliminated = reduced.reidemeister_i_or_ii(anchor);
                    eliminated.sort_by_key(|index| Diagram::label(*index));
                    let eliminated_labels: Vec<_> =
                        eliminated.into_iter().map(Diagram::label).collect();
                    let mut expected_labels = added_labels.clone();
                    expected_labels.sort();
                    if eliminated_labels == expected_labels
                        && reduced.state_sha256() == expected_before
                    {
                        accepted = true;
                        break;
                    }
                }
                if !accepted {
                    return Err(format!(
                        "planar move {move_index} is not an exact inverse RI/RII expansion"
                    )
                    .into());
                }
                diagram = target;
            }
            "Orient" => {
                if certificate.format != CERTIFICATE_FORMAT_V1 {
                    return Err("orientation refresh requires certificate format v1".into());
                }
                let mut target = Diagram::from_state(
                    step.state
                        .as_deref()
                        .ok_or("Orient has no exact target state")?,
                )?;
                while target.crossings.len() < diagram.crossings.len() {
                    let index = target.crossings.len();
                    target.crossings.push(Crossing {
                        sign: 0,
                        adjacent: [Endpoint {
                            crossing: index,
                            index: 0,
                        }; 4],
                        alive: false,
                    });
                }
                let signs = step.signs.as_ref().ok_or("Orient has no sign map")?;
                let alive_labels: HashSet<_> = diagram
                    .crossings
                    .iter()
                    .enumerate()
                    .filter(|(_, crossing)| crossing.alive)
                    .map(|(index, _)| Diagram::label(index))
                    .collect();
                if signs.keys().cloned().collect::<HashSet<_>>() != alive_labels
                    || signs.values().any(|sign| !matches!(sign, -1 | 1))
                {
                    return Err("Orient sign map does not cover the live diagram".into());
                }
                if !target.is_endpoint_reindexing_of(&diagram) {
                    return Err("Orient is not a cyclic endpoint reindexing".into());
                }
                for (index, after) in target.crossings.iter().enumerate() {
                    if after.alive && signs.get(&Diagram::label(index)).copied() != Some(after.sign)
                    {
                        return Err("Orient misstated a crossing sign".into());
                    }
                }
                diagram = target;
            }
            other => return Err(format!("unknown planar move `{other}`").into()),
        }
        if diagram.alive_count() != step.remaining_crossings {
            return Err(format!("planar move {move_index} crossing count mismatch").into());
        }
        let actual_after = diagram.state_sha256();
        if actual_after != parse_sha256(&step.after_sha256)? {
            return Err(format!(
                "planar move {move_index} output checkpoint mismatch: expected {}, got {}; state={}",
                step.after_sha256,
                sha256_hex(actual_after),
                diagram.state_json(),
            )
            .into());
        }
    }
    if diagram.alive_count() != 0 {
        return Err("planar certificate does not end at the empty unknot diagram".into());
    }
    if final_checkpoint.crossings != 0
        || final_checkpoint.unlinked_unknot_components != 1
        || parse_sha256(&final_checkpoint.state_sha256)? != diagram.state_sha256()
    {
        return Err("planar certificate final checkpoint is inconsistent".into());
    }
    Ok(())
}

fn sha256_hex(value: RepKey) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
pub(crate) const TREFOIL_TEST_CERTIFICATE: &[u8] = br#"{
  "format":"unknotdb-labelled-reidemeister-trace-v0",
  "input":{"ordinary_artin_braid_word":[-1,1,1],"crossings":3},
  "moves":[
    {"move":"RII","at":"x0","eliminated":["x0","x2"],"before_sha256":"750911b0ab53786826c7baca05cff858b3a60db52385789fae0ed5ba1be8b0a1","after_sha256":"fd226ee81006a9d77ea2794e019dd5f81bf4e8f1f7032bbb4031bd53c2f59307","remaining_crossings":1},
    {"move":"RI","at":"x1","eliminated":["x1"],"before_sha256":"fd226ee81006a9d77ea2794e019dd5f81bf4e8f1f7032bbb4031bd53c2f59307","after_sha256":"4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945","remaining_crossings":0}
  ],
  "final":{"crossings":0,"state_sha256":"4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945","unlinked_unknot_components":1}
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_replays_a_python_generated_trefoil_certificate() {
        let source = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![-1, 1, 1],
        };
        let certificate = TREFOIL_TEST_CERTIFICATE;
        verify_certificate(&source, certificate).unwrap();
        let checkpoint_tamper = String::from_utf8(certificate.to_vec())
            .unwrap()
            .replace("750911b0", "850911b0");
        assert!(verify_certificate(&source, checkpoint_tamper.as_bytes()).is_err());
        let mut tampered = certificate.to_vec();
        *tampered.last_mut().unwrap() = b'!';
        assert!(verify_certificate(&source, &tampered).is_err());
    }
}
