struct Declaration {
    identifier: &'static str,
    conforms_to: &'static [&'static str],
    mime_types: &'static [&'static str],
    filename_extensions: &'static [&'static str],
}

const fn declare(
    identifier: &'static str,
    conforms_to: &'static [&'static str],
    mime_types: &'static [&'static str],
    filename_extensions: &'static [&'static str],
) -> Declaration {
    Declaration { identifier, conforms_to, mime_types, filename_extensions }
}

const DECLARATIONS: &[Declaration] = &[
    declare("public.item", &[], &[], &[]),
    declare("public.content", &[], &[], &[]),
    declare("public.data", &["public.item"], &["application/octet-stream"], &[]),
    declare("public.audiovisual-content", &["public.data", "public.content"], &[], &[]),
    declare("public.movie", &["public.audiovisual-content"], &[], &[]),
    declare("public.video", &["public.movie"], &[], &[]),
    declare("public.audio", &["public.audiovisual-content"], &[], &[]),
    declare("public.mpeg-4", &["public.movie"], &["video/mp4", "video/mp4v"], &["mp4", "mpg4"]),
    declare("com.apple.m4v-video", &["public.mpeg-4"], &["video/x-m4v"], &["m4v"]),
    declare("com.apple.quicktime-movie", &["public.movie"], &["video/quicktime"], &["mov", "qt"]),
    declare("public.mpeg", &["public.movie"], &["video/mpeg", "video/mpg"], &["mpg", "mpeg", "m75", "m15"]),
    declare("public.mpeg-2-video", &["public.movie"], &["video/mpeg2"], &["m2v"]),
    declare("public.mpeg-2-transport-stream", &["public.movie"], &["video/mp2t"], &["ts"]),
    declare("public.avi", &["public.movie"], &["video/avi", "video/msvideo", "video/x-msvideo"], &["avi", "vfw"]),
    declare("public.3gpp", &["public.movie"], &["video/3gpp", "audio/3gpp"], &["3gp", "3gpp", "sdv"]),
    declare("public.3gpp2", &["public.movie"], &["video/3gpp2", "audio/3gpp2"], &["3g2", "3gp2"]),
    declare("org.webmproject.webm", &["public.movie"], &["video/webm"], &["webm"]),
    declare("com.apple.m4a-audio", &["public.mpeg-4-audio"], &["audio/x-m4a", "audio/m4a"], &["m4a"]),
    declare("public.mpeg-4-audio", &["public.mpeg-4", "public.audio"], &["audio/mp4", "audio/mpeg4"], &["m4b", "m4p"]),
    declare("public.mp3", &["public.audio"], &["audio/mpeg", "audio/mpeg3", "audio/mpg", "audio/mp3", "audio/x-mpeg3", "audio/x-mp3"], &["mp3"]),
    declare("public.aac-audio", &["public.audio"], &["audio/aac", "audio/x-aac"], &["aac", "adts"]),
    declare("public.aiff-audio", &["public.audio"], &["audio/aiff", "audio/x-aiff"], &["aiff", "aif"]),
    declare("com.microsoft.waveform-audio", &["public.audio"], &["audio/vnd.wave", "audio/wav", "audio/wave", "audio/x-wav"], &["wav", "wave"]),
    declare("com.apple.coreaudio-format", &["public.audio"], &["audio/x-caf"], &["caf"]),
    declare("org.xiph.flac", &["public.audio"], &["audio/flac", "audio/x-flac"], &["flac"]),
    declare("public.image", &["public.data", "public.content"], &[], &[]),
    declare("public.jpeg", &["public.image"], &["image/jpeg", "image/jpg"], &["jpeg", "jpg", "jpe"]),
    declare("public.png", &["public.image"], &["image/png"], &["png"]),
    declare("com.compuserve.gif", &["public.image"], &["image/gif"], &["gif"]),
    declare("public.tiff", &["public.image"], &["image/tiff"], &["tiff", "tif"]),
    declare("public.heif", &["public.image"], &["image/heif"], &["heif"]),
    declare("public.heic", &["public.heif"], &["image/heic"], &["heic"]),
    declare("org.webmproject.webp", &["public.image"], &["image/webp"], &["webp"]),
    declare("public.text", &["public.data", "public.content"], &[], &[]),
    declare("public.plain-text", &["public.text"], &["text/plain"], &["txt", "text"]),
    declare("public.json", &["public.text"], &["application/json"], &["json"]),
    declare("com.adobe.pdf", &["public.data", "public.content"], &["application/pdf"], &["pdf"]),
    declare("public.url", &["public.data"], &[], &[]),
    declare("public.file-url", &["public.url"], &[], &[]),
];

/// one declared type, compared and hashed by identifier
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct UTType {
    identifier: &'static str,
}

impl UTType {
    fn declaration(&self) -> Option<&'static Declaration> {
        DECLARATIONS.iter().find(|declaration| declaration.identifier == self.identifier)
    }

    const fn named(identifier: &'static str) -> Self {
        Self { identifier }
    }

    /// type for an identifier, None when no declaration here claims it
    pub fn from_identifier(identifier: &str) -> Option<UTType> {
        DECLARATIONS
            .iter()
            .find(|declaration| declaration.identifier.eq_ignore_ascii_case(identifier))
            .map(|declaration| Self::named(declaration.identifier))
    }

    /// type for a mime type, None if unknown
    /// case and parameters are ignored so video/mp4 with codecs is video/mp4
    pub fn from_mime_type(mime_type: &str) -> Option<UTType> {
        let essence = mime_type.split(';').next().unwrap_or_default();
        let essence = essence.trim_end_matches(' ').trim_start_matches(' ');
        if essence.is_empty() {
            return None;
        }
        DECLARATIONS
            .iter()
            .find(|declaration| declaration.mime_types.iter().any(|tag| tag.eq_ignore_ascii_case(essence)))
            .map(|declaration| Self::named(declaration.identifier))
    }

    /// type for a file extension, no leading dot and case doesnt matter
    pub fn from_filename_extension(filename_extension: &str) -> Option<UTType> {
        let filename_extension = filename_extension.strip_prefix('.').unwrap_or(filename_extension);
        if filename_extension.is_empty() {
            return None;
        }
        DECLARATIONS
            .iter()
            .find(|declaration| {
                declaration.filename_extensions.iter().any(|tag| tag.eq_ignore_ascii_case(filename_extension))
            })
            .map(|declaration| Self::named(declaration.identifier))
    }

    /// the apple style identifier of the type
    pub fn identifier(&self) -> String {
        self.identifier.to_owned()
    }

    pub fn preferred_mime_type(&self) -> Option<String> {
        self.declaration()
            .and_then(|declaration| declaration.mime_types.first())
            .map(|tag| (*tag).to_owned())
    }

    pub fn preferred_filename_extension(&self) -> Option<String> {
        self.declaration()
            .and_then(|declaration| declaration.filename_extensions.first())
            .map(|tag| (*tag).to_owned())
    }

    /// true for the type itself or any type it conforms to, transitively
    pub fn conforms_to(&self, other: &UTType) -> bool {
        let mut pending = vec![self.identifier];
        let mut seen: Vec<&'static str> = Vec::new();
        while let Some(identifier) = pending.pop() {
            if identifier == other.identifier {
                return true;
            }
            if seen.contains(&identifier) {
                continue;
            }
            seen.push(identifier);
            if let Some(declaration) = DECLARATIONS.iter().find(|declaration| declaration.identifier == identifier) {
                pending.extend(declaration.conforms_to.iter().copied());
            }
        }
        false
    }

    pub fn item() -> UTType {
        Self::named("public.item")
    }

    pub fn content() -> UTType {
        Self::named("public.content")
    }

    pub fn data() -> UTType {
        Self::named("public.data")
    }

    pub fn audiovisual_content() -> UTType {
        Self::named("public.audiovisual-content")
    }

    pub fn movie() -> UTType {
        Self::named("public.movie")
    }

    pub fn video() -> UTType {
        Self::named("public.video")
    }

    pub fn audio() -> UTType {
        Self::named("public.audio")
    }

    pub fn mpeg4_movie() -> UTType {
        Self::named("public.mpeg-4")
    }

    pub fn mpeg() -> UTType {
        Self::named("public.mpeg")
    }

    pub fn quick_time_movie() -> UTType {
        Self::named("com.apple.quicktime-movie")
    }

    pub fn mpeg4_audio() -> UTType {
        Self::named("public.mpeg-4-audio")
    }

    pub fn mp3() -> UTType {
        Self::named("public.mp3")
    }

    pub fn image() -> UTType {
        Self::named("public.image")
    }

    pub fn jpeg() -> UTType {
        Self::named("public.jpeg")
    }

    pub fn png() -> UTType {
        Self::named("public.png")
    }

    pub fn gif() -> UTType {
        Self::named("com.compuserve.gif")
    }

    pub fn tiff() -> UTType {
        Self::named("public.tiff")
    }

    pub fn heif() -> UTType {
        Self::named("public.heif")
    }

    pub fn heic() -> UTType {
        Self::named("public.heic")
    }

    pub fn text() -> UTType {
        Self::named("public.text")
    }

    pub fn plain_text() -> UTType {
        Self::named("public.plain-text")
    }

    pub fn json() -> UTType {
        Self::named("public.json")
    }

    pub fn pdf() -> UTType {
        Self::named("com.adobe.pdf")
    }

    /// a link a provider shares like safari or notes
    /// loaded by ShareItemLoader as a url
    pub fn url() -> UTType {
        Self::named("public.url")
    }

    /// a link to a file on this machine
    pub fn file_url() -> UTType {
        Self::named("public.file-url")
    }
}

#[cfg(test)]
pub(crate) fn declared_identifiers() -> Vec<&'static str> {
    DECLARATIONS.iter().map(|declaration| declaration.identifier).collect()
}

#[cfg(test)]
pub(crate) fn named_parents() -> Vec<&'static str> {
    DECLARATIONS.iter().flat_map(|declaration| declaration.conforms_to.iter().copied()).collect()
}
