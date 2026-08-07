//! What the bundled faces actually declare about themselves.
//!
//! `assets/fonts/CONTEXT.txt` names the hazard: Inter follows the RIBBI
//! convention, so only Regular, Italic, Bold and BoldItalic may share a legacy
//! family name. Light, Medium, SemiBold and Black each carry their OWN legacy
//! family ("Inter Medium", "Inter SemiBold", …) and put the real family in the
//! typographic name record, ID 16. A text stack that reads only ID 1 will not
//! find weight 500 or 600 under family "Inter", and it does not fail — it
//! silently answers Regular, so every semibold label in the product draws at
//! regular with nothing in a log.
//!
//! These tests read the `name` table out of the shipped bytes. They pin the
//! hazard; they cannot tell you what a text stack does with it.
//!
//! WHY THERE IS NO TEST HERE THAT RESOLVES A FONT. `TestAppContext` installs
//! `gpui::NoopTextSystem`, which returns `FontId(1)` for every query including
//! `"NotAFamilyAtAll"`. A resolution test written against it passes or fails for
//! reasons that have nothing to do with fonts. The real check runs at startup
//! against the real text system, in `rise_ui::fonts::load_bundled_fonts`, and
//! reports through `FontLoad::misresolved`.

use std::path::PathBuf;

use gpui::FontWeight;
use rise_ui::fonts::BUNDLED_FACES;

const LEGACY_FAMILY: u16 = 1;
const TYPOGRAPHIC_FAMILY: u16 = 16;

fn face_bytes(asset_path: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("assets")
        .join(asset_path);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn be16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]))
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([
        *bytes.get(at)?,
        *bytes.get(at + 1)?,
        *bytes.get(at + 2)?,
        *bytes.get(at + 3)?,
    ]))
}

fn table(bytes: &[u8], tag: &[u8; 4]) -> Option<(usize, usize)> {
    let count = be16(bytes, 4)? as usize;
    for index in 0..count {
        let entry = 12 + index * 16;
        if bytes.get(entry..entry + 4)? == tag {
            let offset = be32(bytes, entry + 8)? as usize;
            let length = be32(bytes, entry + 12)? as usize;
            return Some((offset, length));
        }
    }
    None
}

/// The value of one `name` record, decoded from whichever encoding it uses.
///
/// Windows records are UTF-16BE; Macintosh Roman records are single-byte and
/// ASCII for anything Inter puts in them.
fn name_record(bytes: &[u8], name_id: u16) -> Option<String> {
    let (offset, _) = table(bytes, b"name")?;
    let count = be16(bytes, offset + 2)? as usize;
    let storage = offset + be16(bytes, offset + 4)? as usize;

    for index in 0..count {
        let record = offset + 6 + index * 12;
        if be16(bytes, record + 6)? != name_id {
            continue;
        }

        let platform = be16(bytes, record)?;
        let length = be16(bytes, record + 8)? as usize;
        let start = storage + be16(bytes, record + 10)? as usize;
        let raw = bytes.get(start..start + length)?;

        return Some(match platform {
            3 => {
                let units: Vec<u16> = raw
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                    .collect();
                String::from_utf16_lossy(&units)
            }
            _ => raw.iter().map(|byte| *byte as char).collect(),
        });
    }
    None
}

/// The family a text stack sees depends on which record it reads.
fn families(asset_path: &str) -> (String, Option<String>) {
    let bytes = face_bytes(asset_path);
    (
        name_record(&bytes, LEGACY_FAMILY).expect("every sfnt has name ID 1"),
        name_record(&bytes, TYPOGRAPHIC_FAMILY),
    )
}

#[test]
fn every_face_belongs_to_one_typographic_family() {
    for face in BUNDLED_FACES {
        let (legacy, typographic) = families(face.asset_path);
        let effective = typographic.unwrap_or(legacy);

        assert_eq!(
            effective, "Inter",
            "{} is not part of the family the theme asks for",
            face.asset_path
        );
    }
}

/// This is the trap, asserted rather than described. When it stops being true —
/// because the release changed, or because someone swapped the typeface — the
/// startup check in `load_bundled_fonts` and the family-per-weight table it
/// guards can be simplified. Until then they are load-bearing.
#[test]
fn the_non_ribbi_weights_still_carry_their_own_legacy_family() {
    let ribbi = [FontWeight::NORMAL, FontWeight::BOLD];

    for face in BUNDLED_FACES {
        let (legacy, typographic) = families(face.asset_path);

        if ribbi.contains(&face.weight) {
            assert_eq!(
                legacy, "Inter",
                "{} is RIBBI and should share the legacy family",
                face.asset_path
            );
        } else {
            assert_ne!(
                legacy, "Inter",
                "{} no longer needs its own legacy family; the workaround in \
                 rise_theme::Typography can go",
                face.asset_path
            );
            assert_eq!(
                typographic.as_deref(),
                Some("Inter"),
                "{} has no typographic family record, so nothing can put it \
                 back in the Inter family",
                face.asset_path
            );
        }
    }
}

#[test]
fn the_bundled_faces_are_the_weights_the_file_names_claim() {
    for face in BUNDLED_FACES {
        let bytes = face_bytes(face.asset_path);
        let (os2, _) = table(&bytes, b"OS/2").expect("every sfnt has an OS/2 table");
        let us_weight_class = be16(&bytes, os2 + 4).expect("OS/2 is longer than four bytes");

        assert_eq!(
            f32::from(us_weight_class),
            face.weight.0,
            "{} declares weight {us_weight_class}",
            face.asset_path
        );
    }
}
