use rise_core::foundation::foundation_text::character_boundaries;
use rise_core::foundation::ns_range::{NSRange, byte_to_utf16_offset};

pub enum TextInputCaret {}

impl TextInputCaret {
    pub fn carried(selection: NSRange, from: &str, to: &str) -> NSRange {
        let old_text: Vec<u16> = from.encode_utf16().collect();
        let new_text: Vec<u16> = to.encode_utf16().collect();

        let prefix = Self::common_prefix(&old_text, &new_text);
        let suffix = Self::common_suffix(&old_text, &new_text, prefix);
        let removed = old_text.len() - prefix - suffix;
        let inserted = new_text.len() - prefix - suffix;

        let mut location = selection.location as isize;
        if selection.location >= prefix + removed {
            location += inserted as isize - removed as isize;
        } else if selection.location > prefix {
            location = (prefix + inserted) as isize;
        }

        let clamped = location.clamp(0, new_text.len() as isize) as usize;
        let location = Self::snapped(clamped, to, new_text.len());
        let length = selection.length.min(new_text.len() - location);
        NSRange::new(location, length)
    }

    fn common_prefix(lhs: &[u16], rhs: &[u16]) -> usize {
        lhs.iter().zip(rhs).take_while(|(l, r)| l == r).count()
    }

    fn common_suffix(lhs: &[u16], rhs: &[u16], prefix: usize) -> usize {
        let limit = lhs.len().min(rhs.len()) - prefix;
        lhs.iter().rev().zip(rhs.iter().rev()).take(limit).take_while(|(l, r)| l == r).count()
    }

    /// start of the character that covers location
    fn snapped(location: usize, text: &str, length: usize) -> usize {
        if location == 0 || location >= length {
            return location;
        }
        character_boundaries(text)
            .into_iter()
            .map(|byte| byte_to_utf16_offset(text, byte))
            .take_while(|&start| start <= location)
            .last()
            .unwrap_or(location)
    }
}
