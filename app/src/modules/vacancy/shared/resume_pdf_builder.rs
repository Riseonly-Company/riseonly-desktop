use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, Font, FontId, FontWeight, LineFragment, TextRun, TextSystem, WindowTextSystem, font, px};
use image::codecs::jpeg::JpegEncoder;
use image::RgbImage;
use rise_core::foundation::foundation_url::Url;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::{URLRequest, URLSession};
use rise_catalog::VacancyGender;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::tr;
use rise_theme::FONT_FAMILY;
use rise_ui::fonts::BUNDLED_FACES;

use crate::modules::vacancy::shared::resume_option_catalog::ResumeOptionCatalog;
use crate::modules::vacancy::shared::ui_graphics_pdf_renderer::{
    CGRect, CGSize, PdfColor, PdfFontId, PdfGlyph, PdfImageId, PdfJpegImage, UIBezierPath, UIGraphicsPDFRenderer,
    UIGraphicsPDFRendererContext,
};
use crate::modules::vacancy::stores::vacancy_actions::vacancy_types::{ResumeModel, WorkExperienceModel};

const IMAGE_TIMEOUT: Duration = Duration::from_secs(12);
const IMAGE_JPEG_QUALITY: u8 = 90;
const APP_LOGO_ASSET: &str = "images/AppLogo.png";
const BIRTH_DATE_INPUT_LOCALE: &str = "en_US_POSIX";
const BIRTH_DATE_INPUT_FORMAT: &str = "yyyy-MM-dd";
const BIRTH_DATE_INPUT_LENGTH: usize = 10;
const BIRTH_DATE_OUTPUT_FORMAT: &str = "dd.MM.yyyy";
const LOCATION_PIN: &str = "\u{1F4CD}";
const PERIOD_SEPARATOR: &str = " \u{2014} ";
const COMPANY_PERIOD_SEPARATOR: &str = "  \u{00B7}  ";

const PAGE_SIZE: CGSize = CGSize::new(595.0, 842.0);
const PAGE_WIDTH: f32 = PAGE_SIZE.width;
const PAGE_HEIGHT: f32 = PAGE_SIZE.height;
const MARGIN: f32 = 44.0;
const ACCENT: PdfColor = PdfColor::rgb(0.0, 0.55, 0.9);
const INK: PdfColor = PdfColor::white(0.12);
const SUB_INK: PdfColor = PdfColor::white(0.42);

/// system font of a size and weight, resolved against the bundled family
#[derive(Clone, Copy, Debug, PartialEq)]
struct UIFont {
    size: f32,
    weight: FontWeight,
}

impl UIFont {
    const fn system_font(of_size: f32, weight: FontWeight) -> Self {
        Self { size: of_size, weight }
    }
}

const HEAVY: FontWeight = FontWeight::BLACK;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResumePdfBuilderTextAlign {
    Left,
    Center,
}

/// images fetched before drawing
struct ResumePdfBuilderImages {
    avatar: Option<PdfJpegImage>,
    logos: HashMap<usize, PdfJpegImage>,
    app_logo: Option<PdfJpegImage>,
}

pub enum ResumePdfBuilder {}

impl ResumePdfBuilder {
    /// builds the pdf, images load off the main thread and drawing happens on it
    pub fn build(resume: ResumeModel, completion: impl FnOnce(Option<Vec<u8>>, &mut App) + 'static, cx: &mut App) {
        let app_logo_data = cx.asset_source().load(APP_LOGO_ASSET).ok().flatten().map(|data| data.into_owned());
        let avatar_url = resume.avatar.clone();
        let logo_urls: Vec<Option<String>> =
            resume.work_experience.iter().map(|experience| experience.company_logo_url.clone()).collect();
        cx.spawn(async move |cx| {
            let images = RiseAccountRuntime::current()
                .spawn(async move {
                    let avatar = Self::load_image(Some(avatar_url.as_str())).await;
                    let mut logos = HashMap::new();
                    for (index, url) in logo_urls.iter().enumerate() {
                        if let Some(url) = url
                            && !url.is_empty()
                            && let Some(image) = Self::load_image(Some(url.as_str())).await
                        {
                            logos.insert(index, image);
                        }
                    }
                    let app_logo = app_logo_data.as_deref().and_then(Self::jpeg_image);
                    ResumePdfBuilderImages { avatar, logos, app_logo }
                })
                .await
                .unwrap_or(ResumePdfBuilderImages { avatar: None, logos: HashMap::new(), app_logo: None });
            cx.update(|cx| {
                let data = Self::render(&resume, images, cx);
                completion(Some(data), cx);
            });
        })
        .detach();
    }

    async fn load_image(url_string: Option<&str>) -> Option<PdfJpegImage> {
        let url_string = url_string.filter(|url| !url.is_empty())?;
        let url = Url::from_string(url_string)?;
        let mut request = URLRequest::new(url);
        request.timeout_interval = IMAGE_TIMEOUT;
        let (data, _) = URLSession::shared().data_for(request).await.ok()?;
        Self::jpeg_image(&data)
    }

    fn jpeg_image(data: &[u8]) -> Option<PdfJpegImage> {
        let decoded = image::load_from_memory(data).ok()?.into_rgba8();
        let (width, height) = decoded.dimensions();
        let mut flattened = RgbImage::new(width, height);
        for (x, y, pixel) in decoded.enumerate_pixels() {
            let [red, green, blue, alpha] = pixel.0;
            let coverage = u32::from(alpha);
            let blend = |channel: u8| -> u8 {
                let mixed = (u32::from(channel) * coverage + 255 * (255 - coverage)) / 255;
                u8::try_from(mixed).unwrap_or(u8::MAX)
            };
            flattened.put_pixel(x, y, image::Rgb([blend(red), blend(green), blend(blue)]));
        }
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, IMAGE_JPEG_QUALITY)
            .encode_image(&flattened)
            .ok()?;
        Some(PdfJpegImage { width, height, jpeg })
    }

    fn render(resume: &ResumeModel, images: ResumePdfBuilderImages, cx: &App) -> Vec<u8> {
        let renderer = UIGraphicsPDFRenderer::new(CGRect::new(0.0, 0.0, PAGE_WIDTH, PAGE_HEIGHT));
        let typesetter = ResumePdfTypesetter::new(cx);
        renderer.pdf_data(|ctx| {
            let avatar = images.avatar.map(|image| ctx.register_image(image));
            let logos: HashMap<usize, PdfImageId> =
                images.logos.into_iter().map(|(index, image)| (index, ctx.register_image(image))).collect();
            let app_logo = images.app_logo.map(|image| ctx.register_image(image));

            let mut pager = Pager::new(ctx, typesetter, PAGE_SIZE, MARGIN);
            pager.begin_page();

            Self::draw_header(&mut pager, resume, avatar);

            Self::draw_section(&mut pager, &tr("vacancy_resume_section_personal").to_uppercase());
            Self::draw_key_values(&mut pager, &Self::contact_pairs(resume));

            if !resume.professional_summary.is_empty() {
                Self::draw_section(&mut pager, &tr("vacancy_resume_section_summary").to_uppercase());
                Self::draw_paragraph(&mut pager, &resume.professional_summary);
            }

            if !resume.work_experience.is_empty() {
                Self::draw_section(&mut pager, &tr("vacancy_experience_section").to_uppercase());
                for (index, experience) in resume.work_experience.iter().enumerate() {
                    Self::draw_experience(&mut pager, experience, logos.get(&index).copied());
                }
            }

            if !resume.skills.is_empty() {
                Self::draw_section(&mut pager, &tr("vacancy_field_skills").to_uppercase());
                Self::draw_chips(&mut pager, &resume.skills);
            }

            Self::draw_footer(&mut pager, app_logo);
        })
    }

    fn draw_header(pager: &mut Pager<'_>, resume: &ResumeModel, avatar: Option<PdfImageId>) {
        let band_height = 150.0;
        pager.ctx.set_fill(ACCENT);
        pager.ctx.fill_path(UIBezierPath::Rect(CGRect::new(0.0, 0.0, PAGE_WIDTH, band_height)));

        let avatar_size = 84.0;
        let avatar_rect = CGRect::new(MARGIN, 36.0, avatar_size, avatar_size);
        pager.ctx.save_g_state();
        pager.ctx.add_clip(UIBezierPath::Oval(avatar_rect));
        match avatar {
            Some(avatar) => pager.ctx.draw_image(avatar, avatar_rect),
            None => {
                pager.ctx.set_fill(PdfColor::WHITE.with_alpha_component(0.25));
                pager.ctx.fill_path(UIBezierPath::Rect(avatar_rect));
                let full_name = resume.full_name();
                let initials: String = full_name
                    .split(' ')
                    .filter(|part| !part.is_empty())
                    .take(2)
                    .filter_map(|part| part.chars().next())
                    .collect();
                pager.draw(
                    &initials.to_uppercase(),
                    UIFont::system_font(30.0, FontWeight::BOLD),
                    PdfColor::WHITE,
                    avatar_rect.offset_by(0.0, 22.0),
                    ResumePdfBuilderTextAlign::Center,
                );
            }
        }
        pager.ctx.restore_g_state();
        pager.ctx.set_stroke(PdfColor::WHITE.with_alpha_component(0.6));
        pager.ctx.stroke_path(UIBezierPath::Oval(avatar_rect), 2.0);

        let text_x = MARGIN + avatar_size + 18.0;
        let text_w = PAGE_WIDTH - text_x - MARGIN;
        let full_name = resume.full_name();
        let name = if full_name.is_empty() { tr("vacancy_resume_create_title") } else { full_name };
        pager.draw(
            &name,
            UIFont::system_font(24.0, FontWeight::BOLD),
            PdfColor::WHITE,
            CGRect::new(text_x, 44.0, text_w, 32.0),
            ResumePdfBuilderTextAlign::Left,
        );
        if !resume.profession.is_empty() {
            pager.draw(
                &resume.profession,
                UIFont::system_font(15.0, FontWeight::SEMIBOLD),
                PdfColor::WHITE.with_alpha_component(0.92),
                CGRect::new(text_x, 78.0, text_w, 22.0),
                ResumePdfBuilderTextAlign::Left,
            );
        }
        let location = resume.location_display();
        if !location.is_empty() {
            pager.draw(
                &format!("{LOCATION_PIN} {location}"),
                UIFont::system_font(12.0, FontWeight::NORMAL),
                PdfColor::WHITE.with_alpha_component(0.85),
                CGRect::new(text_x, 102.0, text_w, 18.0),
                ResumePdfBuilderTextAlign::Left,
            );
        }

        pager.y = band_height + 26.0;
    }

    fn draw_section(pager: &mut Pager<'_>, title: &str) {
        pager.ensure(40.0);
        let y = pager.y;
        pager.ctx.set_fill(ACCENT);
        pager.ctx.fill_path(UIBezierPath::RoundedRect(CGRect::new(MARGIN, y + 1.0, 3.0, 14.0), 1.5));
        let width = pager.content_width() - 10.0;
        pager.draw(
            title,
            UIFont::system_font(12.0, HEAVY),
            ACCENT,
            CGRect::new(MARGIN + 10.0, y, width, 16.0),
            ResumePdfBuilderTextAlign::Left,
        );
        pager.y += 24.0;
    }

    fn draw_key_values(pager: &mut Pager<'_>, pairs: &[(String, String)]) {
        let label_font = UIFont::system_font(11.0, FontWeight::SEMIBOLD);
        let value_font = UIFont::system_font(13.0, FontWeight::NORMAL);
        for (key, value) in pairs.iter().filter(|(_, value)| !value.is_empty()) {
            let label = key.to_uppercase();
            let label_h = pager.text_height(&label, label_font, 150.0);
            let value_w = pager.content_width() - 160.0;
            let value_h = pager.text_height(value, value_font, value_w);
            let row_h = f32::max(20.0, label_h.max(value_h) + 6.0);
            pager.ensure(row_h);
            let y = pager.y;
            pager.draw(&label, label_font, SUB_INK, CGRect::new(MARGIN, y, 150.0, label_h), ResumePdfBuilderTextAlign::Left);
            pager.draw(
                value,
                value_font,
                INK,
                CGRect::new(MARGIN + 160.0, y - 1.0, value_w, value_h),
                ResumePdfBuilderTextAlign::Left,
            );
            pager.y += row_h;
        }
        pager.y += 6.0;
    }

    fn draw_paragraph(pager: &mut Pager<'_>, text: &str) {
        let font = UIFont::system_font(13.0, FontWeight::NORMAL);
        let w = pager.content_width();
        let mut remaining = text.to_owned();
        while !remaining.is_empty() {
            let avail = pager.remaining() - 10.0;
            if avail < 30.0 {
                pager.begin_page();
            }
            let max_height = pager.remaining() - 10.0;
            let (fit, rest) = Self::split_text(pager, &remaining, font, w, max_height);
            let h = pager.text_height(&fit, font, w);
            let y = pager.y;
            pager.draw(&fit, font, INK, CGRect::new(MARGIN, y, w, h), ResumePdfBuilderTextAlign::Left);
            pager.y += h + 4.0;
            if !rest.is_empty() {
                pager.begin_page();
            }
            remaining = rest;
        }
        pager.y += 8.0;
    }

    fn draw_experience(pager: &mut Pager<'_>, exp: &WorkExperienceModel, logo: Option<PdfImageId>) {
        pager.ensure(66.0);
        let start_y = pager.y;
        let logo_size = 38.0;
        let logo_rect = CGRect::new(MARGIN, start_y, logo_size, logo_size);

        let path = UIBezierPath::RoundedRect(logo_rect, 9.0);
        pager.ctx.save_g_state();
        pager.ctx.add_clip(path);
        match logo {
            Some(logo) => pager.ctx.draw_image(logo, logo_rect),
            None => {
                pager.ctx.set_fill(PdfColor::white(0.92));
                pager.ctx.fill_path(UIBezierPath::Rect(logo_rect));
                let letter: String = exp.company.chars().take(1).collect::<String>().to_uppercase();
                pager.draw(
                    &letter,
                    UIFont::system_font(18.0, FontWeight::BOLD),
                    SUB_INK,
                    logo_rect.offset_by(0.0, 9.0),
                    ResumePdfBuilderTextAlign::Center,
                );
            }
        }
        pager.ctx.restore_g_state();

        let tx = MARGIN + logo_size + 12.0;
        let tw = pager.content_width() - logo_size - 12.0;
        let title = if exp.position.is_empty() { &exp.company } else { &exp.position };
        pager.draw(
            title,
            UIFont::system_font(14.0, FontWeight::SEMIBOLD),
            INK,
            CGRect::new(tx, start_y, tw, 18.0),
            ResumePdfBuilderTextAlign::Left,
        );
        let end = if exp.is_current { tr("vacancy_experience_current") } else { exp.to_date.clone().unwrap_or_default() };
        let period = [exp.from_date.clone(), end].into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(PERIOD_SEPARATOR);
        let subtitle = [exp.company.clone(), period]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(COMPANY_PERIOD_SEPARATOR);
        pager.draw(
            &subtitle,
            UIFont::system_font(11.0, FontWeight::NORMAL),
            SUB_INK,
            CGRect::new(tx, start_y + 19.0, tw, 16.0),
            ResumePdfBuilderTextAlign::Left,
        );

        pager.y = start_y + 40.0;
        if !exp.description.is_empty() {
            let font = UIFont::system_font(12.0, FontWeight::NORMAL);
            let h = pager.text_height(&exp.description, font, tw);
            pager.ensure(h + 6.0);
            let y = pager.y;
            pager.draw(
                &exp.description,
                font,
                PdfColor::white(0.3),
                CGRect::new(tx, y, tw, h),
                ResumePdfBuilderTextAlign::Left,
            );
            pager.y += h;
        }
        pager.y += 14.0;
    }

    fn draw_chips(pager: &mut Pager<'_>, items: &[String]) {
        let font = UIFont::system_font(12.0, FontWeight::MEDIUM);
        let mut x = MARGIN;
        let chip_h = 24.0;
        let gap = 8.0;
        pager.ensure(chip_h + 4.0);
        for item in items {
            let text_w = pager.text_width(item, font) + 22.0;
            if x + text_w > MARGIN + pager.content_width() {
                x = MARGIN;
                pager.y += chip_h + gap;
                pager.ensure(chip_h + 4.0);
            }
            let chip = CGRect::new(x, pager.y, text_w, chip_h);
            pager.ctx.set_fill(ACCENT.with_alpha_component(0.1));
            pager.ctx.fill_path(UIBezierPath::RoundedRect(chip, 12.0));
            pager.draw(item, font, ACCENT, chip.offset_by(0.0, 4.0), ResumePdfBuilderTextAlign::Center);
            x += text_w + gap;
        }
        pager.y += chip_h + 14.0;
    }

    fn draw_footer(pager: &mut Pager<'_>, app_logo: Option<PdfImageId>) {
        let y = PAGE_HEIGHT - 44.0;
        pager.ctx.set_stroke(PdfColor::white(0.88));
        pager.ctx.stroke_line((MARGIN, y), (PAGE_WIDTH - MARGIN, y), 0.5);

        let mut logo_x = MARGIN;
        if let Some(logo) = app_logo {
            let logo_rect = CGRect::new(MARGIN, y + 8.0, 18.0, 18.0);
            pager.ctx.save_g_state();
            pager.ctx.add_clip(UIBezierPath::RoundedRect(logo_rect, 4.0));
            pager.ctx.draw_image(logo, logo_rect);
            pager.ctx.restore_g_state();
            logo_x = MARGIN + 24.0;
        }
        pager.draw(
            &tr("vacancy_resume_pdf_footer"),
            UIFont::system_font(10.0, FontWeight::MEDIUM),
            SUB_INK,
            CGRect::new(logo_x, y + 11.0, PAGE_WIDTH - logo_x - MARGIN, 14.0),
            ResumePdfBuilderTextAlign::Left,
        );
    }

    fn contact_pairs(r: &ResumeModel) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = Vec::new();
        if !r.phone.is_empty() {
            pairs.push((tr("vacancy_resume_phone"), r.phone.clone()));
        }
        let location = r.location_display();
        if !location.is_empty() {
            pairs.push((tr("vacancy_field_city"), location));
        }
        let salary = r.formatted_salary();
        if salary != tr("vacancy_salary_negotiable") {
            pairs.push((tr("vacancy_resume_salary"), salary));
        }
        if !r.birth_date.is_empty() {
            pairs.push((tr("vacancy_resume_birth_date"), Self::format_birth_date(&r.birth_date)));
        }
        if !r.gender.is_empty() {
            pairs.push((tr("vacancy_resume_gender"), Self::gender_label(&r.gender)));
        }
        if !r.education.is_empty() {
            pairs.push((tr("vacancy_resume_education"), Self::education_label(&r.education)));
        }
        if !r.experience_level.is_empty() {
            pairs.push((tr("vacancy_resume_exp_level"), Self::exp_level_label(&r.experience_level)));
        }
        pairs.push((tr("vacancy_relocation_available"), if r.ready_to_relocation { tr("yes") } else { tr("no") }));
        pairs.push((tr("vacancy_resume_army"), if r.ready_to_army { tr("yes") } else { tr("no") }));
        pairs
    }

    fn format_birth_date(raw: &str) -> String {
        let mut in_fmt = DateFormatter::new();
        in_fmt.set_locale(BIRTH_DATE_INPUT_LOCALE);
        in_fmt.set_date_format(BIRTH_DATE_INPUT_FORMAT);
        let prefix: String = raw.chars().take(BIRTH_DATE_INPUT_LENGTH).collect();
        let Some(date) = in_fmt.date_from(&prefix) else {
            return raw.to_owned();
        };
        let mut out_fmt = DateFormatter::new();
        out_fmt.set_date_format(BIRTH_DATE_OUTPUT_FORMAT);
        out_fmt.string_from(&date)
    }

    fn gender_label(g: &str) -> String {
        if g == VacancyGender::Male.wire() {
            tr("vacancy_gender_male")
        } else if g == VacancyGender::Female.wire() {
            tr("vacancy_gender_female")
        } else {
            g.to_owned()
        }
    }

    fn education_label(e: &str) -> String {
        tr(ResumeOptionCatalog::education_key(e))
    }

    fn exp_level_label(e: &str) -> String {
        tr(ResumeOptionCatalog::experience_key(e))
    }

    fn split_text(pager: &mut Pager<'_>, text: &str, font: UIFont, width: f32, max_height: f32) -> (String, String) {
        if pager.text_height(text, font, width) <= max_height {
            return (text.to_owned(), String::new());
        }
        let words: Vec<&str> = text.split(' ').collect();
        let mut fit = String::new();
        let mut idx = 0;
        while idx < words.len() {
            let candidate = if fit.is_empty() { words[idx].to_owned() } else { format!("{fit} {}", words[idx]) };
            if pager.text_height(&candidate, font, width) > max_height {
                break;
            }
            fit = candidate;
            idx += 1;
        }
        let rest = words[idx..].join(" ");
        (if fit.is_empty() { text.to_owned() } else { fit }, rest)
    }
}

/// one bundled face as the text system resolved it, embedded on first use
struct ResumePdfFace {
    weight: FontWeight,
    font: Font,
    font_id: FontId,
    pdf_font: Option<PdfFontId>,
    embedded_glyph_ids: bool,
}

const GLYPH_ID_PROBE: &str = "A\u{44f}";

/// text measuring and drawing over the app text system,
/// lines wrap at word boundaries and line height is ascent plus descent
struct ResumePdfTypesetter {
    text_system: Arc<TextSystem>,
    layout: WindowTextSystem,
    faces: Vec<ResumePdfFace>,
    face_data: Vec<(FontWeight, Vec<u8>)>,
}

impl ResumePdfTypesetter {
    fn new(cx: &App) -> Self {
        let text_system = cx.text_system().clone();
        let source = cx.asset_source().clone();
        let face_data = BUNDLED_FACES
            .iter()
            .filter_map(|face| {
                let data = source.load(face.asset_path).ok().flatten()?;
                (!data.is_empty()).then(|| (face.weight, data.into_owned()))
            })
            .collect();
        Self { layout: WindowTextSystem::new(text_system.clone()), text_system, faces: Vec::new(), face_data }
    }

    fn face_index(&mut self, weight: FontWeight) -> usize {
        if let Some(index) = self.faces.iter().position(|face| face.weight == weight) {
            return index;
        }
        let font = Font { weight, ..font(FONT_FAMILY) };
        let font_id = self.text_system.resolve_font(&font);
        self.faces.push(ResumePdfFace { weight, font, font_id, pdf_font: None, embedded_glyph_ids: false });
        self.faces.len() - 1
    }

    fn pdf_font(&mut self, index: usize, ctx: &mut UIGraphicsPDFRendererContext) -> Option<PdfFontId> {
        let face = self.faces.get(index)?;
        if let Some(pdf_font) = face.pdf_font {
            return Some(pdf_font);
        }
        let weight = face.weight;
        let font_id = face.font_id;
        let data = self.face_data.iter().find(|(face_weight, _)| *face_weight == weight).map(|(_, data)| data.clone())?;
        let units_per_em = self.text_system.units_per_em(font_id) as f32;
        let ascent = f32::from(self.text_system.ascent(font_id, px(units_per_em)));
        let descent = -f32::from(self.text_system.descent(font_id, px(units_per_em))).abs();
        let embedded_glyph_ids = self.shaper_face_differs(index, &data);
        let pdf_font = ctx.register_font(data, ascent, descent, units_per_em);
        if let Some(face) = self.faces.get_mut(index) {
            face.pdf_font = Some(pdf_font);
            face.embedded_glyph_ids = embedded_glyph_ids;
        }
        Some(pdf_font)
    }

    fn shaper_face_differs(&self, index: usize, data: &[u8]) -> bool {
        let Ok(embedded) = ttf_parser::Face::parse(data, 0) else {
            return false;
        };
        let Some(face) = self.faces.get(index) else {
            return false;
        };
        if u32::from(embedded.units_per_em()) != self.text_system.units_per_em(face.font_id) {
            return true;
        }
        let size = f32::from(embedded.units_per_em());
        let layout =
            self.layout.layout_line(GLYPH_ID_PROBE, px(size), &[Self::run(&face.font, GLYPH_ID_PROBE.len())], None);
        let shaped: Vec<(usize, u32)> = layout
            .runs
            .iter()
            .filter(|run| run.font_id == face.font_id)
            .flat_map(|run| run.glyphs.iter())
            .map(|glyph| (glyph.index, glyph.id.0))
            .collect();
        GLYPH_ID_PROBE.char_indices().any(|(at, character)| {
            let embedded_id = embedded.glyph_index(character).map(|id| u32::from(id.0));
            let shaped_id = shaped.iter().find(|(glyph_at, _)| *glyph_at == at).map(|(_, id)| *id);
            embedded_id != shaped_id
        })
    }

    fn embedded_face(&self, index: usize) -> Option<ttf_parser::Face<'_>> {
        let face = self.faces.get(index)?;
        if !face.embedded_glyph_ids {
            return None;
        }
        let (_, data) = self.face_data.iter().find(|(weight, _)| *weight == face.weight)?;
        ttf_parser::Face::parse(data, 0).ok()
    }

    fn line_height(&mut self, font: UIFont) -> (f32, f32) {
        let index = self.face_index(font.weight);
        let font_id = self.faces[index].font_id;
        let ascent = f32::from(self.text_system.ascent(font_id, px(font.size)));
        let descent = f32::from(self.text_system.descent(font_id, px(font.size))).abs();
        (ascent, ascent + descent)
    }

    fn lines(&mut self, text: &str, font: UIFont, width: f32) -> Vec<String> {
        let index = self.face_index(font.weight);
        let gpui_font = self.faces[index].font.clone();
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            if paragraph.is_empty() {
                lines.push(String::new());
                continue;
            }
            let mut wrapper = self.text_system.line_wrapper(gpui_font.clone(), px(font.size));
            let fragments = [LineFragment::text(paragraph)];
            let boundaries: Vec<usize> = wrapper.wrap_line(&fragments, px(width.max(1.0))).map(|boundary| boundary.ix).collect();
            let mut start = 0;
            for end in boundaries.into_iter().chain(std::iter::once(paragraph.len())) {
                if end < start || !paragraph.is_char_boundary(end) {
                    continue;
                }
                lines.push(paragraph[start..end].trim_end_matches(' ').to_owned());
                start = end;
            }
        }
        lines
    }

    fn text_height(&mut self, text: &str, font: UIFont, width: f32) -> f32 {
        let (_, line_height) = self.line_height(font);
        let count = self.lines(text, font, width).len().max(1) as f32;
        (count * line_height).ceil()
    }

    fn text_width(&mut self, text: &str, font: UIFont) -> f32 {
        let index = self.face_index(font.weight);
        let run = Self::run(&self.faces[index].font, text.len());
        f32::from(self.layout.layout_line(text, px(font.size), &[run], None).width).ceil()
    }

    fn draw(
        &mut self,
        ctx: &mut UIGraphicsPDFRendererContext,
        text: &str,
        font: UIFont,
        color: PdfColor,
        rect: CGRect,
        align: ResumePdfBuilderTextAlign,
    ) {
        let index = self.face_index(font.weight);
        let Some(pdf_font) = self.pdf_font(index, ctx) else {
            return;
        };
        let font_id = self.faces[index].font_id;
        let gpui_font = self.faces[index].font.clone();
        let (ascent, line_height) = self.line_height(font);
        let mut top = rect.y;
        for (line_index, line) in self.lines(text, font, rect.width).into_iter().enumerate() {
            if line_index > 0 && top + line_height > rect.y + rect.height + 0.5 {
                break;
            }
            if !line.is_empty() {
                let layout = self.layout.layout_line(&line, px(font.size), &[Self::run(&gpui_font, line.len())], None);
                let line_width = f32::from(layout.width);
                let x = match align {
                    ResumePdfBuilderTextAlign::Left => rect.x,
                    ResumePdfBuilderTextAlign::Center => rect.x + (rect.width - line_width) / 2.0,
                };
                let embedded = self.embedded_face(index);
                let glyphs: Vec<PdfGlyph> = layout
                    .runs
                    .iter()
                    .filter(|run| run.font_id == font_id)
                    .flat_map(|run| run.glyphs.iter())
                    .filter_map(|glyph| {
                        let character = line.get(glyph.index..).and_then(|rest| rest.chars().next());
                        let id = match &embedded {
                            Some(embedded) => embedded.glyph_index(character?)?.0,
                            None => u16::try_from(glyph.id.0).ok()?,
                        };
                        Some(PdfGlyph { id, x: f32::from(glyph.position.x), character })
                    })
                    .collect();
                ctx.draw_glyphs(pdf_font, font.size, color, x, top + ascent, &glyphs);
            }
            top += line_height;
        }
    }

    fn run(font: &Font, len: usize) -> TextRun {
        TextRun {
            len,
            font: font.clone(),
            color: gpui::black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        }
    }
}

/// page flow of the pdf builder
struct Pager<'a> {
    ctx: &'a mut UIGraphicsPDFRendererContext,
    text: ResumePdfTypesetter,
    page_size: CGSize,
    margin: f32,
    y: f32,
}

impl<'a> Pager<'a> {
    fn new(
        ctx: &'a mut UIGraphicsPDFRendererContext,
        text: ResumePdfTypesetter,
        page_size: CGSize,
        margin: f32,
    ) -> Self {
        Self { ctx, text, page_size, margin, y: 0.0 }
    }

    fn content_width(&self) -> f32 {
        self.page_size.width - self.margin * 2.0
    }

    /// space left on the current page
    fn remaining(&self) -> f32 {
        self.page_size.height - self.margin - 24.0 - self.y
    }

    /// starts a new page
    fn begin_page(&mut self) {
        self.ctx.begin_page();
        self.y = self.margin;
    }

    /// starts a new page if less than this much space is left
    fn ensure(&mut self, space: f32) {
        if self.remaining() < space {
            self.begin_page();
        }
    }

    /// draws text in a rect with font, color and alignment
    fn draw(&mut self, text: &str, font: UIFont, color: PdfColor, rect: CGRect, align: ResumePdfBuilderTextAlign) {
        self.text.draw(self.ctx, text, font, color, rect, align);
    }

    fn text_height(&mut self, text: &str, font: UIFont, width: f32) -> f32 {
        self.text.text_height(text, font, width)
    }

    /// width the text takes on one line
    fn text_width(&mut self, text: &str, font: UIFont) -> f32 {
        self.text.text_width(text, font)
    }
}
