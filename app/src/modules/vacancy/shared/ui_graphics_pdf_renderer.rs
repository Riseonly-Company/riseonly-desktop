use std::collections::BTreeMap;
use std::fmt::Write as _;

const POINTS_PER_EM_UNIT: f32 = 1000.0;
const BEZIER_CIRCLE_FACTOR: f32 = 0.552_284_8;
const SYMBOLIC_FONT_FLAG: i32 = 32;
const DEFAULT_STEM_V: i32 = 80;
const PDF_HEADER: &[u8] = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n";

/// rectangle with origin and size
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CGRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl CGRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    /// same rect moved by dx and dy
    pub fn offset_by(self, dx: f32, dy: f32) -> Self {
        Self { x: self.x + dx, y: self.y + dy, ..self }
    }
}

/// width and height pair
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CGSize {
    pub width: f32,
    pub height: f32,
}

impl CGSize {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

/// device rgb color with alpha
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfColor {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl PdfColor {
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);

    /// opaque color from rgb parts
    pub const fn rgb(red: f32, green: f32, blue: f32) -> Self {
        Self { red, green, blue, alpha: 1.0 }
    }

    /// opaque gray from one white value
    pub const fn white(white: f32) -> Self {
        Self::rgb(white, white, white)
    }

    /// same color with another alpha
    pub const fn with_alpha_component(self, alpha: f32) -> Self {
        Self { alpha, ..self }
    }
}

/// the three shapes the builder draws
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UIBezierPath {
    Rect(CGRect),
    RoundedRect(CGRect, f32),
    Oval(CGRect),
}

/// one embedded face, registered before drawing
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PdfFontId(usize);

/// one embedded image, registered before drawing
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PdfImageId(usize);

/// decoded image re-encoded as baseline jpeg, what the pdf carries as is
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfJpegImage {
    pub width: u32,
    pub height: u32,
    pub jpeg: Vec<u8>,
}

/// shaped glyph,
/// its id in the face,
/// x from the run origin and the char it shows for copy and search
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfGlyph {
    pub id: u16,
    pub x: f32,
    pub character: Option<char>,
}

struct PdfFontFace {
    data: Vec<u8>,
    ascent: f32,
    descent: f32,
    used: BTreeMap<u16, char>,
}

/// drawing context of the pdf renderer
pub struct UIGraphicsPDFRendererContext {
    page_width: f32,
    page_height: f32,
    pages: Vec<String>,
    fonts: Vec<PdfFontFace>,
    images: Vec<PdfJpegImage>,
    alphas: Vec<f32>,
    fill: PdfColor,
    stroke: PdfColor,
}

impl UIGraphicsPDFRendererContext {
    fn new(page_width: f32, page_height: f32) -> Self {
        Self {
            page_width,
            page_height,
            pages: Vec::new(),
            fonts: Vec::new(),
            images: Vec::new(),
            alphas: Vec::new(),
            fill: PdfColor::white(0.0),
            stroke: PdfColor::white(0.0),
        }
    }

    pub fn register_font(&mut self, data: Vec<u8>, ascent: f32, descent: f32, units_per_em: f32) -> PdfFontId {
        let scale = if units_per_em > 0.0 { POINTS_PER_EM_UNIT / units_per_em } else { 1.0 };
        self.fonts.push(PdfFontFace { data, ascent: ascent * scale, descent: descent * scale, used: BTreeMap::new() });
        PdfFontId(self.fonts.len() - 1)
    }

    pub fn register_image(&mut self, image: PdfJpegImage) -> PdfImageId {
        self.images.push(image);
        PdfImageId(self.images.len() - 1)
    }

    /// starts a new page
    pub fn begin_page(&mut self) {
        self.pages.push(String::new());
        self.fill = PdfColor::white(0.0);
        self.stroke = PdfColor::white(0.0);
    }

    pub fn save_g_state(&mut self) {
        self.op("q");
    }

    /// restores the saved graphics state
    pub fn restore_g_state(&mut self) {
        self.op("Q");
    }

    pub fn set_fill(&mut self, color: PdfColor) {
        self.fill = color;
    }

    pub fn set_stroke(&mut self, color: PdfColor) {
        self.stroke = color;
    }

    pub fn fill_path(&mut self, path: UIBezierPath) {
        let color = self.fill;
        self.op("q");
        self.apply_alpha(color.alpha);
        self.op(&format!("{} {} {} rg", num(color.red), num(color.green), num(color.blue)));
        self.append_path(path);
        self.op("f");
        self.op("Q");
    }

    pub fn stroke_path(&mut self, path: UIBezierPath, line_width: f32) {
        let color = self.stroke;
        self.op("q");
        self.apply_alpha(color.alpha);
        self.op(&format!("{} {} {} RG {} w", num(color.red), num(color.green), num(color.blue), num(line_width)));
        self.append_path(path);
        self.op("S");
        self.op("Q");
    }

    pub fn stroke_line(&mut self, from: (f32, f32), to: (f32, f32), line_width: f32) {
        let color = self.stroke;
        self.op("q");
        self.apply_alpha(color.alpha);
        self.op(&format!(
            "{} {} {} RG {} w {} {} m {} {} l S",
            num(color.red),
            num(color.green),
            num(color.blue),
            num(line_width),
            num(from.0),
            num(self.page_height - from.1),
            num(to.0),
            num(self.page_height - to.1),
        ));
        self.op("Q");
    }

    /// clips to a path until the matching restore_g_state
    pub fn add_clip(&mut self, path: UIBezierPath) {
        self.append_path(path);
        self.op("W n");
    }

    pub fn draw_image(&mut self, image: PdfImageId, rect: CGRect) {
        self.op(&format!(
            "q {} 0 0 {} {} {} cm /Im{} Do Q",
            num(rect.width),
            num(rect.height),
            num(rect.x),
            num(self.page_height - rect.y - rect.height),
            image.0
        ));
    }

    pub fn draw_glyphs(
        &mut self,
        font: PdfFontId,
        size: f32,
        color: PdfColor,
        origin_x: f32,
        baseline_y: f32,
        glyphs: &[PdfGlyph],
    ) {
        if glyphs.is_empty() {
            return;
        }
        if let Some(face) = self.fonts.get_mut(font.0) {
            for glyph in glyphs {
                if let Some(character) = glyph.character {
                    face.used.entry(glyph.id).or_insert(character);
                } else {
                    face.used.entry(glyph.id).or_insert(char::REPLACEMENT_CHARACTER);
                }
            }
        }
        let y = self.page_height - baseline_y;
        let mut body = String::new();
        let _ = write!(body, "BT /F{} {} Tf {} {} {} rg", font.0, num(size), num(color.red), num(color.green), num(color.blue));
        for glyph in glyphs {
            let _ = write!(body, " 1 0 0 1 {} {} Tm <{:04X}> Tj", num(origin_x + glyph.x), num(y), glyph.id);
        }
        body.push_str(" ET");
        self.op("q");
        self.apply_alpha(color.alpha);
        self.op(&body);
        self.op("Q");
    }

    fn apply_alpha(&mut self, alpha: f32) {
        if alpha >= 1.0 {
            return;
        }
        let index = match self.alphas.iter().position(|known| (known - alpha).abs() < f32::EPSILON) {
            Some(index) => index,
            None => {
                self.alphas.push(alpha);
                self.alphas.len() - 1
            }
        };
        self.op(&format!("/GS{index} gs"));
    }

    fn append_path(&mut self, path: UIBezierPath) {
        let h = self.page_height;
        let ops = match path {
            UIBezierPath::Rect(rect) => {
                format!("{} {} {} {} re", num(rect.x), num(h - rect.y - rect.height), num(rect.width), num(rect.height))
            }
            UIBezierPath::RoundedRect(rect, radius) => {
                let r = radius.min(rect.width / 2.0).min(rect.height / 2.0).max(0.0);
                Self::rounded_rect_ops(rect, r, r, h)
            }
            UIBezierPath::Oval(rect) => Self::rounded_rect_ops(rect, rect.width / 2.0, rect.height / 2.0, h),
        };
        self.op(&ops);
    }

    fn rounded_rect_ops(rect: CGRect, rx: f32, ry: f32, page_height: f32) -> String {
        let left = rect.x;
        let right = rect.x + rect.width;
        let top = page_height - rect.y;
        let bottom = page_height - rect.y - rect.height;
        let kx = rx * BEZIER_CIRCLE_FACTOR;
        let ky = ry * BEZIER_CIRCLE_FACTOR;
        let mut out = String::new();
        let _ = write!(out, "{} {} m", num(left + rx), num(top));
        let _ = write!(out, " {} {} l", num(right - rx), num(top));
        let _ = write!(
            out,
            " {} {} {} {} {} {} c",
            num(right - rx + kx),
            num(top),
            num(right),
            num(top - ry + ky),
            num(right),
            num(top - ry)
        );
        let _ = write!(out, " {} {} l", num(right), num(bottom + ry));
        let _ = write!(
            out,
            " {} {} {} {} {} {} c",
            num(right),
            num(bottom + ry - ky),
            num(right - rx + kx),
            num(bottom),
            num(right - rx),
            num(bottom)
        );
        let _ = write!(out, " {} {} l", num(left + rx), num(bottom));
        let _ = write!(
            out,
            " {} {} {} {} {} {} c",
            num(left + rx - kx),
            num(bottom),
            num(left),
            num(bottom + ry - ky),
            num(left),
            num(bottom + ry)
        );
        let _ = write!(out, " {} {} l", num(left), num(top - ry));
        let _ = write!(
            out,
            " {} {} {} {} {} {} c h",
            num(left),
            num(top - ry + ky),
            num(left + rx - kx),
            num(top),
            num(left + rx),
            num(top)
        );
        out
    }

    fn op(&mut self, operator: &str) {
        if self.pages.is_empty() {
            self.begin_page();
        }
        if let Some(page) = self.pages.last_mut() {
            page.push_str(operator);
            page.push('\n');
        }
    }

    fn finish(self) -> Vec<u8> {
        let mut writer = PdfObjectWriter::default();
        let catalog_id = writer.reserve();
        let pages_id = writer.reserve();

        let mut font_entries = String::new();
        for (index, face) in self.fonts.iter().enumerate() {
            if face.used.is_empty() {
                continue;
            }
            let font_id = Self::write_font(&mut writer, index, face);
            let _ = write!(font_entries, "/F{index} {font_id} 0 R ");
        }
        let mut image_entries = String::new();
        for (index, image) in self.images.iter().enumerate() {
            let header = format!(
                "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>",
                image.width,
                image.height,
                image.jpeg.len()
            );
            let image_id = writer.add_stream(&header, &image.jpeg);
            let _ = write!(image_entries, "/Im{index} {image_id} 0 R ");
        }
        let mut alpha_entries = String::new();
        for (index, alpha) in self.alphas.iter().enumerate() {
            let state_id = writer.add(&format!("<< /Type /ExtGState /ca {} /CA {} >>", num(*alpha), num(*alpha)));
            let _ = write!(alpha_entries, "/GS{index} {state_id} 0 R ");
        }
        let resources_id = writer.add(&format!(
            "<< /Font << {font_entries}>> /XObject << {image_entries}>> /ExtGState << {alpha_entries}>> >>"
        ));

        let mut kids = String::new();
        let pages = if self.pages.is_empty() { vec![String::new()] } else { self.pages };
        let page_count = pages.len();
        for content in pages {
            let content_id = writer.add_stream(&format!("<< /Length {} >>", content.len()), content.as_bytes());
            let page_id = writer.add(&format!(
                "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {} {}] /Resources {resources_id} 0 R /Contents {content_id} 0 R >>",
                num(self.page_width),
                num(self.page_height)
            ));
            let _ = write!(kids, "{page_id} 0 R ");
        }
        writer.set(pages_id, &format!("<< /Type /Pages /Kids [{kids}] /Count {page_count} >>"));
        writer.set(catalog_id, &format!("<< /Type /Catalog /Pages {pages_id} 0 R >>"));
        writer.finish(catalog_id)
    }

    fn write_font(writer: &mut PdfObjectWriter, index: usize, face: &PdfFontFace) -> usize {
        let base_font = format!("RiseFace{index}");
        let file_id = writer.add_stream(
            &format!("<< /Length {} /Length1 {} >>", face.data.len(), face.data.len()),
            &face.data,
        );
        let descriptor_id = writer.add(&format!(
            "<< /Type /FontDescriptor /FontName /{base_font} /Flags {SYMBOLIC_FONT_FLAG} /FontBBox [0 {} {} {}] /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV {DEFAULT_STEM_V} /FontFile2 {file_id} 0 R >>",
            num(face.descent),
            num(POINTS_PER_EM_UNIT),
            num(face.ascent),
            num(face.ascent),
            num(face.descent),
            num(face.ascent)
        ));
        let cid_id = writer.add(&format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{base_font} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {descriptor_id} 0 R /CIDToGIDMap /Identity >>"
        ));
        let cmap = Self::to_unicode_cmap(&face.used);
        let cmap_id = writer.add_stream(&format!("<< /Length {} >>", cmap.len()), cmap.as_bytes());
        writer.add(&format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /{base_font} /Encoding /Identity-H /DescendantFonts [{cid_id} 0 R] /ToUnicode {cmap_id} 0 R >>"
        ))
    }

    fn to_unicode_cmap(used: &BTreeMap<u16, char>) -> String {
        let mut out = String::from(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
        );
        let entries: Vec<(&u16, &char)> = used.iter().collect();
        for chunk in entries.chunks(100) {
            let _ = writeln!(out, "{} beginbfchar", chunk.len());
            for (glyph, character) in chunk {
                let mut units = [0u16; 2];
                let encoded: String = character.encode_utf16(&mut units).iter().map(|unit| format!("{unit:04X}")).collect();
                let _ = writeln!(out, "<{glyph:04X}> <{encoded}>");
            }
            out.push_str("endbfchar\n");
        }
        out.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        out
    }
}

#[derive(Default)]
struct PdfObjectWriter {
    objects: Vec<Vec<u8>>,
}

impl PdfObjectWriter {
    fn reserve(&mut self) -> usize {
        self.objects.push(Vec::new());
        self.objects.len()
    }

    fn set(&mut self, id: usize, body: &str) {
        if let Some(slot) = self.objects.get_mut(id - 1) {
            *slot = body.as_bytes().to_vec();
        }
    }

    fn add(&mut self, body: &str) -> usize {
        self.objects.push(body.as_bytes().to_vec());
        self.objects.len()
    }

    fn add_stream(&mut self, header: &str, data: &[u8]) -> usize {
        let mut body = Vec::with_capacity(header.len() + data.len() + 32);
        body.extend_from_slice(header.as_bytes());
        body.extend_from_slice(b"\nstream\n");
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        self.objects.push(body);
        self.objects.len()
    }

    fn finish(self, root: usize) -> Vec<u8> {
        let mut out = Vec::from(PDF_HEADER);
        let mut offsets = Vec::with_capacity(self.objects.len());
        for (index, body) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref_offset = out.len();
        let mut xref = format!("xref\n0 {}\n0000000000 65535 f \n", self.objects.len() + 1);
        for offset in offsets {
            let _ = write!(xref, "{offset:010} 00000 n \n");
        }
        let _ = write!(
            xref,
            "trailer\n<< /Size {} /Root {root} 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n",
            self.objects.len() + 1
        );
        out.extend_from_slice(xref.as_bytes());
        out
    }
}

/// pdf renderer
pub struct UIGraphicsPDFRenderer {
    width: f32,
    height: f32,
}

impl UIGraphicsPDFRenderer {
    /// renderer for pages of these bounds
    pub fn new(bounds: CGRect) -> Self {
        Self { width: bounds.width, height: bounds.height }
    }

    pub fn pdf_data(&self, actions: impl FnOnce(&mut UIGraphicsPDFRendererContext)) -> Vec<u8> {
        let mut context = UIGraphicsPDFRendererContext::new(self.width, self.height);
        actions(&mut context);
        context.finish()
    }
}

fn num(value: f32) -> String {
    if !value.is_finite() {
        return "0".to_owned();
    }
    let rounded = (value * 1000.0).round() / 1000.0;
    if rounded.fract() == 0.0 { format!("{}", rounded as i64) } else { format!("{rounded}") }
}
