use serde::{Deserialize, Serialize};

pub mod progress;

/// Axis-aligned bounding rectangle in visual page points (origin at top-left, +X right, +Y down).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct RectPt {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl RectPt {
    pub const fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub fn from_points(ax: f32, ay: f32, bx: f32, by: f32) -> Self {
        Self {
            x0: ax.min(bx),
            y0: ay.min(by),
            x1: ax.max(bx),
            y1: ay.max(by),
        }
    }

    pub fn width(&self) -> f32 {
        (self.x1 - self.x0).max(0.0)
    }

    pub fn height(&self) -> f32 {
        (self.y1 - self.y0).max(0.0)
    }

    pub fn area(&self) -> f32 {
        self.width() * self.height()
    }

    pub fn center_x(&self) -> f32 {
        0.5 * (self.x0 + self.x1)
    }

    pub fn center_y(&self) -> f32 {
        0.5 * (self.y0 + self.y1)
    }

    pub fn union(&self, other: &Self) -> Self {
        Self {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }

    pub fn intersection(&self, other: &Self) -> Option<Self> {
        let x0 = self.x0.max(other.x0);
        let y0 = self.y0.max(other.y0);
        let x1 = self.x1.min(other.x1);
        let y1 = self.y1.min(other.y1);
        if x1 > x0 && y1 > y0 {
            Some(Self { x0, y0, x1, y1 })
        } else {
            None
        }
    }

    pub fn vertical_overlap_ratio(&self, other: &Self) -> f32 {
        let top = self.y0.max(other.y0);
        let bot = self.y1.min(other.y1);
        let overlap = (bot - top).max(0.0);
        let min_h = self.height().min(other.height()).max(0.1);
        overlap / min_h
    }

    pub fn horizontal_overlap_ratio(&self, other: &Self) -> f32 {
        let left = self.x0.max(other.x0);
        let right = self.x1.min(other.x1);
        let overlap = (right - left).max(0.0);
        let min_w = self.width().min(other.width()).max(0.1);
        overlap / min_w
    }
}

/// 2D affine matrix `[a, b, c, d, e, f]` mapping `(x, y)` to `(a*x + c*y + e, b*x + d*y + f)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Affine2D {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Default for Affine2D {
    fn default() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }
}

/// RGBA color (0–255 per channel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RgbaColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Default for RgbaColor {
    fn default() -> Self {
        Self::BLACK
    }
}

impl RgbaColor {
    pub const BLACK: Self = Self { r: 0, g: 0, b: 0, a: 255 };
    pub const WHITE: Self = Self { r: 255, g: 255, b: 255, a: 255 };

    pub fn composited_over_white(&self) -> (u8, u8, u8) {
        if self.a == 255 {
            (self.r, self.g, self.b)
        } else {
            let a = self.a as u16;
            let blend = |c: u8| ((c as u16 * a + 255 * (255 - a) + 127) / 255) as u8;
            (blend(self.r), blend(self.g), blend(self.b))
        }
    }

    pub fn to_hex_rgb(&self) -> String {
        let (r, g, b) = self.composited_over_white();
        format!("{r:02X}{g:02X}{b:02X}")
    }

    pub fn is_near_white(&self) -> bool {
        let (r, g, b) = self.composited_over_white();
        r >= 245 && g >= 245 && b >= 245
    }

    pub fn is_near_black(&self) -> bool {
        let (r, g, b) = self.composited_over_white();
        r <= 30 && g <= 30 && b <= 30
    }
}

/// PDF text rendering mode (`Tr`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RenderMode {
    #[default]
    Fill,
    Stroke,
    FillAndStroke,
    Invisible,
}

/// Extracted glyph in visual page coordinates (top-left origin).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlyphIr {
    /// Decoded Unicode string (may be >1 char for ligatures or ActualText).
    pub unicode: String,
    /// Raw glyph ID in the font.
    pub glyph_id: u16,
    /// Visual bounding box on the page (points, origin top-left).
    pub bbox: RectPt,
    /// Baseline Y coordinate in visual page points (top-down).
    pub baseline_y: f32,
    /// Effective font size in points.
    pub font_size_pt: f32,
    /// Horizontal advance in points.
    pub advance_pt: f32,
    /// PDF character spacing (`Tc`) active when this glyph was drawn, in points.
    pub char_spacing_pt: f32,
    /// PDF word spacing (`Tw`) active when this glyph was drawn, in points.
    pub word_spacing_pt: f32,
    /// Horizontal scaling factor (`Tz / 100.0`, default 1.0; Chrome/Skia often uses 0.75 matrix).
    pub horiz_scale: f32,
    /// Rotation angle in degrees (0 for horizontal LTR text).
    pub angle_deg: f32,
    /// Font identifier in `PageIr::fonts`.
    pub font_idx: usize,
    /// Fill color.
    pub color: RgbaColor,
    /// Render mode (Fill, Stroke, Invisible for OCR layers).
    pub render_mode: RenderMode,
    /// Optional Marked Content ID (`MCID`).
    pub mcid: Option<i32>,
    /// Optional `ActualText` override from a parent `/Span` marked-content sequence.
    pub actual_text: Option<String>,
    /// True if glyph falls inside the current clipping region.
    pub is_visible_in_clip: bool,
}

/// Font metadata extracted from the PDF.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FontIr {
    pub postscript_name: String,
    pub family_name: String,
    pub is_bold: bool,
    pub is_italic: bool,
    pub is_monospace: bool,
    pub is_serif: bool,
    pub is_embedded_subset: bool,
    /// Factor the extractor multiplied this font's `GlyphIr::font_size_pt` by, for fonts whose
    /// glyphs draw smaller than their nominal size; divide by it to get the size set in the PDF.
    #[serde(default)]
    pub size_scale: Option<f32>,
    /// The font program embedded in the PDF (TrueType/OpenType, or bare CFF), when there is one.
    /// Only kept while a document is open (direct editing draws with it), never serialized.
    #[serde(skip)]
    pub program: Option<FontProgram>,
    /// Advance the PDF sets for each glyph drawn in this font (`/W`, `/Widths`; 1000 units per
    /// em), by glyph id. Producers may round these away from the program's own metrics.
    #[serde(skip)]
    pub glyph_widths: std::collections::BTreeMap<u16, f32>,
    /// Outline of each glyph drawn in this font, by glyph id (character code for Type 3 fonts):
    /// what the editor draws fonts a browser cannot load with (Type 1, CFF, Type 3), and what a
    /// missing font is compared to installed ones with.
    #[serde(skip)]
    pub shapes: std::collections::BTreeMap<u16, GlyphShape>,
}

/// A glyph's outline in em space: 1000 units per em, y up, origin at the pen position.
#[derive(Debug, Clone, PartialEq)]
pub struct GlyphShape {
    pub unicode: char,
    /// Advance in em units.
    pub advance: f32,
    pub path: Vec<PathOp>,
}

/// One drawing command of a [`GlyphShape`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathOp {
    MoveTo([f32; 2]),
    LineTo([f32; 2]),
    QuadTo([f32; 4]),
    CurveTo([f32; 6]),
    Close,
}

/// Bytes of a font program embedded in a PDF.
#[derive(Clone, PartialEq)]
pub struct FontProgram(pub std::sync::Arc<[u8]>);

impl std::fmt::Debug for FontProgram {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FontProgram({} bytes)", self.0.len())
    }
}

/// Extracted vector path (ruling lines, underlines, table borders, background fills, or outline glyphs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PathIr {
    pub bbox: RectPt,
    pub stroke_color: Option<RgbaColor>,
    pub fill_color: Option<RgbaColor>,
    pub stroke_width_pt: f32,
    pub is_axis_aligned_rect: bool,
    pub is_horizontal_line: bool,
    pub is_vertical_line: bool,
    /// Number of closed subpaths/segments (used to detect outline-converted text).
    pub segment_count: usize,
}

/// Extracted raster image or rendered region on a page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageIr {
    pub bbox: RectPt,
    pub pixel_width: u32,
    pub pixel_height: u32,
    /// `"jpeg"` or `"png"`.
    pub format: String,
    /// Encoded image bytes (safe RGB JPEG passthrough or PNG).
    pub data: Vec<u8>,
    pub is_full_page_scan: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnnotKind {
    FreeText,
    Ink,
    Stamp,
    Highlight,
    Link { uri: String },
}

/// Non-widget PDF annotation (Preview Fill & Sign, Acrobat notes, highlights, links).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnnotIr {
    pub kind: AnnotKind,
    pub bbox: RectPt,
    pub text_contents: Option<String>,
    pub color: Option<RgbaColor>,
    pub font_size_pt: Option<f32>,
    pub appearance_png: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldKind {
    Text,
    CheckBox { checked: bool },
    RadioButton { selected: bool },
    Choice,
    Signature,
}

/// Interactive AcroForm field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormFieldIr {
    pub name: String,
    pub kind: FieldKind,
    pub value: Option<String>,
    pub bbox: RectPt,
    pub page_index: usize,
}

/// Intermediate representation of a single PDF page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PageIr {
    pub page_index: usize,
    pub width_pt: f32,
    pub height_pt: f32,
    pub rotation_deg: i32,
    pub fonts: Vec<FontIr>,
    pub glyphs: Vec<GlyphIr>,
    pub paths: Vec<PathIr>,
    pub images: Vec<ImageIr>,
    pub annotations: Vec<AnnotIr>,
    pub form_fields: Vec<FormFieldIr>,
    /// Optional full-page raster PNG (used by fallback rung or EXACT background).
    pub page_raster_png: Option<Vec<u8>>,
    /// Optional render of the page's graphics and images with its horizontal text left out
    /// (PNG or JPEG bytes, see `graphics_format`), used as the background of `Preset::Exact` pages.
    #[serde(default)]
    pub graphics_png: Option<Vec<u8>>,
    /// `"png"` or `"jpeg"` for `graphics_png`.
    #[serde(default)]
    pub graphics_format: String,
    /// The page draws something beyond plain axis-aligned rectangles and rules (images, curves,
    /// gradients, transparency, clipped shapes, rotated text), so it needs a raster background.
    #[serde(default)]
    pub has_complex_graphics: bool,
}

/// Intermediate representation of a full PDF document.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DocIr {
    pub pages: Vec<PageIr>,
    pub has_acroform: bool,
    pub producer: Option<String>,
    pub title: Option<String>,
}

// ============================================================================
// Output Model (`FlowDoc`)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Profile {
    /// Works cleanly in Apple Pages, Microsoft Word, and LibreOffice with 0 Pages warnings.
    #[default]
    Portable,
    /// Specifically tuned for Apple Pages (Mac-only fonts in all 4 rFonts slots, U+202F -> U+00A0, no nested tables).
    Pages,
    /// Tuned for Microsoft Word.
    Word,
    /// Tuned for LibreOffice Writer.
    LibreOffice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Preset {
    /// Default: `AtLeast` leading so +4 pt font enlargement never clips, soft page breaks, cross-page joins.
    #[default]
    Editable,
    /// Strict visual lock: `Exact` leading, hard page breaks per source page.
    Fidele,
    /// Pixel-identical copy: each page's graphics as a background image, with every text block
    /// re-created as live, editable text in a box anchored at its exact PDF position.
    Exact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum OutputFormat {
    #[default]
    Docx,
    Pages,
    Html,
    Markdown,
    Txt,
    SearchablePdf,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LineSpacingRule {
    /// Minimum line height in points (`w:lineRule="atLeast"`). Default in `Editable` mode.
    AtLeastPt(f32),
    /// Proportional multiplier (`1.15`, `1.2`, etc., `w:lineRule="auto"`).
    Multiple(f32),
    /// Fixed line height in points (`w:lineRule="exact"`). Used only in `Fidele` mode.
    ExactPt(f32),
}

impl Default for LineSpacingRule {
    fn default() -> Self {
        Self::Multiple(1.15)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Alignment {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabLeader {
    None,
    Dot,
    Underscore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TabAlignment {
    Left,
    Right,
    Center,
    Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TabStop {
    pub pos_pt: f32,
    pub align: TabAlignment,
    pub leader: TabLeader,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunStyle {
    /// Mapped Mac-safe font family (e.g., `"Helvetica Neue"`, `"Georgia"`, `"Menlo"`).
    pub font_family: String,
    pub font_size_pt: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub color: RgbaColor,
    pub highlight_color: Option<RgbaColor>,
    /// Character tracking in points (preserved from tracked headings so `"FACTURE"` stays visually spaced without spaces between letters).
    pub char_tracking_pt: f32,
    /// Horizontal glyph width scale percentage (e.g. `100` = normal, `95` = 95%).
    pub width_scale_pct: u16,
    pub link_uri: Option<String>,
}

impl Default for RunStyle {
    fn default() -> Self {
        Self {
            font_family: "Helvetica Neue".to_string(),
            font_size_pt: 11.0,
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
            color: RgbaColor::BLACK,
            highlight_color: None,
            char_tracking_pt: 0.0,
            width_scale_pct: 100,
            link_uri: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextRun {
    pub text: String,
    pub style: RunStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListKind {
    Bullet,
    OrderedDecimal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Paragraph {
    pub runs: Vec<TextRun>,
    pub alignment: Alignment,
    pub space_before_pt: f32,
    pub space_after_pt: f32,
    pub left_indent_pt: f32,
    pub right_indent_pt: f32,
    pub first_line_indent_pt: f32,
    pub line_spacing: LineSpacingRule,
    pub heading_level: Option<u8>,
    pub list_kind: Option<ListKind>,
    pub tab_stops: Vec<TabStop>,
    pub page_break_before: bool,
    pub keep_with_next: bool,
    pub auto_hyphenation: bool,
    pub source_bbox: RectPt,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CellBorder {
    pub width_pt: f32,
    pub color: RgbaColor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TableCell {
    pub paragraphs: Vec<Paragraph>,
    pub grid_span: u16,
    /// `None` = normal cell, `Some(true)` = vertical merge start (`restart`), `Some(false)` = continuation.
    pub v_merge: Option<bool>,
    pub width_pt: f32,
    pub shading_color: Option<RgbaColor>,
    pub border_top: Option<CellBorder>,
    pub border_bottom: Option<CellBorder>,
    pub border_left: Option<CellBorder>,
    pub border_right: Option<CellBorder>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
    /// Minimum row height in points (`AtLeast`, never `Exact`, so enlarged text inside cells never clips).
    pub min_height_pt: f32,
    pub is_header_row: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TableGrid {
    pub col_widths_pt: Vec<f32>,
    pub rows: Vec<TableRow>,
    pub left_indent_pt: f32,
    pub space_before_pt: f32,
    pub space_after_pt: f32,
    pub is_borderless_layout: bool,
    pub source_bbox: RectPt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnchoredImage {
    pub media_id: usize,
    pub format: String,
    pub data: Vec<u8>,
    pub bbox_pt: RectPt,
    pub behind_doc: bool,
    pub inline: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextBoxBlock {
    pub bbox_pt: RectPt,
    pub paragraphs: Vec<Paragraph>,
    pub fill_color: Option<RgbaColor>,
    pub border: Option<CellBorder>,
    /// Always true in `EXACT` mode so font substitution or user edits expand the box instead of clipping.
    pub auto_fit_height: bool,
}

/// Rectangle or rule positioned on the page behind the text (`Preset::Exact`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeBlock {
    pub bbox_pt: RectPt,
    pub fill_color: Option<RgbaColor>,
    /// Outline centred on `bbox_pt`'s edges.
    pub stroke: Option<CellBorder>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FlowBlock {
    Paragraph(Paragraph),
    Table(TableGrid),
    Image(AnchoredImage),
    TextBox(TextBoxBlock),
    Shape(ShapeBlock),
    PageBreak,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub page_width_pt: f32,
    pub page_height_pt: f32,
    pub margin_top_pt: f32,
    pub margin_bottom_pt: f32,
    pub margin_left_pt: f32,
    pub margin_right_pt: f32,
    pub column_count: u8,
    pub column_widths_pt: Vec<f32>,
    pub column_space_pt: f32,
    pub header_paragraphs: Vec<Paragraph>,
    pub footer_paragraphs: Vec<Paragraph>,
    pub blocks: Vec<FlowBlock>,
}

impl Default for Section {
    fn default() -> Self {
        Self {
            page_width_pt: 595.28,  // A4 width
            page_height_pt: 841.89, // A4 height
            margin_top_pt: 56.7,
            margin_bottom_pt: 56.7,
            margin_left_pt: 56.7,
            margin_right_pt: 56.7,
            column_count: 1,
            column_widths_pt: Vec::new(),
            column_space_pt: 18.0,
            header_paragraphs: Vec::new(),
            footer_paragraphs: Vec::new(),
            blocks: Vec::new(),
        }
    }
}

/// Complete reconstructed document ready for serialization into DOCX / HTML / MD / TXT / SPDF.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct FlowDoc {
    pub title: Option<String>,
    pub default_font_family: String,
    pub default_font_size_pt: f32,
    pub sections: Vec<Section>,
    pub source_pages: Vec<PageIr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageCase {
    DigitalLetter,
    InvoiceOrStatement,
    MultiColumn,
    FillableAcroForm,
    FlatForm,
    ExactFlyer,
    OutlineText,
    ScannedPage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FallbackRung {
    /// Rung 1: Full layout reconstruction (paragraphs, tables, columns, headers/footers, images).
    FullPipeline = 1,
    /// Rung 2: Safe paragraph flow only (tables/columns disabled).
    SafeParagraphFlow = 2,
    /// Rung 3: Secondary PDF backend extraction.
    SecondaryBackend = 3,
    /// Rung 4: Full-page OCR -> FLOW.
    FullPageOcr = 4,
    /// Rung 5: Page raster image + extracted text below.
    PageImagePlusText = 5,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageReport {
    pub page_index: usize,
    pub detected_case: PageCase,
    pub rung_used: FallbackRung,
    pub glyph_count: usize,
    pub word_count: usize,
    pub table_count: usize,
    pub unmapped_glyph_ratio: f32,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConvertReport {
    pub page_count: usize,
    pub has_acroform: bool,
    pub form_field_count: usize,
    pub pages: Vec<PageReport>,
    pub total_words: usize,
    pub glued_word_fixes: usize,
    pub tracked_heading_fixes: usize,
    pub cmap_fallback_recoveries: usize,
    pub elapsed_ms: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Params {
    pub profile: Profile,
    pub preset: Preset,
    pub format: OutputFormat,
    /// Minimum ratio of line height to font size in `Editable` mode.
    pub min_editable_line_factor: f32,
    /// Enable cross-page continuation table merging.
    pub merge_continuation_tables: bool,
    /// Enable Chrome/Skia U+200B space normalization.
    pub normalize_skia_zwsp: bool,
    /// Enable 2-means Tc/Tw letter-spacing clustering.
    pub cluster_letter_spacing: bool,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            profile: Profile::Pages,
            preset: Preset::Editable,
            format: OutputFormat::Docx,
            min_editable_line_factor: 1.15,
            merge_continuation_tables: true,
            normalize_skia_zwsp: true,
            cluster_letter_spacing: true,
        }
    }
}
