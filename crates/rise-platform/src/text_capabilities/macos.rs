use std::ffi::{c_char, c_void};
use std::ptr;

use super::text_capabilities::TextCoverageUnavailable;
use crate::host_os::HostOs;

type CfTypeRef = *const c_void;
type CfIndex = isize;
type UniChar = u16;
type CgGlyph = u16;

#[repr(C)]
#[derive(Clone, Copy)]
struct CfRange {
    location: CfIndex,
    length: CfIndex,
}

const UI_FONT_SYSTEM: u32 = 2;
const BASE_FONT_SIZE: f64 = 15.0;
const UTF8_ENCODING: u32 = 0x0800_0100;
const NAME_BUFFER: usize = 128;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFStringCreateWithCharacters(
        allocator: CfTypeRef,
        characters: *const UniChar,
        count: CfIndex,
    ) -> CfTypeRef;
    fn CFStringGetRangeOfComposedCharactersAtIndex(string: CfTypeRef, index: CfIndex) -> CfRange;
    fn CFStringGetCString(
        string: CfTypeRef,
        buffer: *mut c_char,
        size: CfIndex,
        encoding: u32,
    ) -> u8;
    fn CFRelease(object: CfTypeRef);
}

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    fn CTFontCreateUIFontForLanguage(kind: u32, size: f64, language: CfTypeRef) -> CfTypeRef;
    fn CTFontCreateForString(current: CfTypeRef, string: CfTypeRef, range: CfRange) -> CfTypeRef;
    fn CTFontCopyPostScriptName(font: CfTypeRef) -> CfTypeRef;
    fn CTFontGetGlyphsForCharacters(
        font: CfTypeRef,
        characters: *const UniChar,
        glyphs: *mut CgGlyph,
        count: CfIndex,
    ) -> bool;
}

/// a corefoundation object this code created or copied, released once on drop
struct Owned(CfTypeRef);

impl Owned {
    fn is_null(&self) -> bool {
        self.0.is_null()
    }
}

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

fn postscript_name(font: &Owned) -> Option<String> {
    let name = Owned(unsafe { CTFontCopyPostScriptName(font.0) });
    if name.is_null() {
        return None;
    }
    let mut buffer = [0 as c_char; NAME_BUFFER];
    let written = unsafe {
        CFStringGetCString(name.0, buffer.as_mut_ptr(), NAME_BUFFER as CfIndex, UTF8_ENCODING)
    };
    if written == 0 {
        return None;
    }
    let text = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) };
    text.to_str().ok().map(str::to_owned)
}

pub(super) fn glyph_coverage(text: &str, host: HostOs) -> Result<bool, TextCoverageUnavailable> {
    let units: Vec<UniChar> = text.encode_utf16().collect();
    if units.is_empty() {
        return Ok(true);
    }
    let failed = |call: &'static str| TextCoverageUnavailable::SystemCallFailed { host, call };
    let total = units.len() as CfIndex;
    let base =
        Owned(unsafe { CTFontCreateUIFontForLanguage(UI_FONT_SYSTEM, BASE_FONT_SIZE, ptr::null()) });
    if base.is_null() {
        return Err(failed("CTFontCreateUIFontForLanguage"));
    }
    let string = Owned(unsafe { CFStringCreateWithCharacters(ptr::null(), units.as_ptr(), total) });
    if string.is_null() {
        return Err(failed("CFStringCreateWithCharacters"));
    }
    let mut location: CfIndex = 0;
    while location < total {
        let rest = CfRange { location, length: total - location };
        let font = Owned(unsafe { CTFontCreateForString(base.0, string.0, rest) });
        if font.is_null() {
            return Err(failed("CTFontCreateForString"));
        }
        if postscript_name(&font).as_deref() == Some("LastResort") {
            return Ok(false);
        }
        let run = unsafe { CFStringGetRangeOfComposedCharactersAtIndex(string.0, location) };
        let start = usize::try_from(run.location).unwrap_or(usize::MAX);
        let end = usize::try_from(run.length).ok().and_then(|length| start.checked_add(length));
        let Some(end) = end.filter(|end| *end > location as usize && *end <= units.len()) else {
            return Err(failed("CFStringGetRangeOfComposedCharactersAtIndex"));
        };
        let characters = &units[start..end];
        let mut glyphs: Vec<CgGlyph> = vec![0; characters.len()];
        let mapped = unsafe {
            CTFontGetGlyphsForCharacters(
                font.0,
                characters.as_ptr(),
                glyphs.as_mut_ptr(),
                characters.len() as CfIndex,
            )
        };
        if !mapped || glyphs.iter().all(|glyph| *glyph == 0) {
            return Ok(false);
        }
        location = end as CfIndex;
    }
    Ok(true)
}
