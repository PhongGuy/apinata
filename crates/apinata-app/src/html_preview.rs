//! An HTML body shown as a simple page. The untrusted page is parsed here by the same parser
//! the kit uses, into a flat arena with no recursion, then written out as Markdown whose text
//! is escaped and whose nesting is capped, so the kit lays out a tree of known, shallow depth.
//! Script, style, title and foreign content are left out, links show as text, and only raster
//! images carried inline as `data:` URLs are shown; no other URL ever reaches a loader.

use std::borrow::Cow;
use std::sync::Arc;

use crate::body_text::ImageKind;
use crate::image_guard::{self, Refusal};
use gpui_kit::component::text::{MarkdownExtensions, TextView};
use gpui_kit::{
    App, Asset, ElementId, ImageCacheError, ImageSource, RenderImage, SharedString, SharedUri,
};
use html5ever::tendril::{StrTendril, TendrilSink as _};
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{
    Attribute, ExpandedName, LocalName, QualName, local_name, namespace_url, ns, parse_document,
};

/// Most nodes a page is parsed into; past it the preview shows the page's start.
pub const MAX_NODES: usize = 50_000;

/// Deepest a page is parsed; past it the preview shows the page's start. The parser scans
/// its open elements for each tag, so this also bounds the parse's time.
pub const MAX_DEPTH: usize = 512;

/// Deepest nesting of lists and quotes the Markdown keeps; deeper ones are flattened.
pub const MAX_NEST: usize = 6;

// The page is fed to the parser in pieces this size, so the node bound stops a page that
// grows quadratically before it costs much.
const CHUNK: usize = 1024;

/// A page ready for the preview.
pub struct Page {
    /// The page as Markdown.
    pub markdown: String,
    /// Whether the page passed [`MAX_NODES`] or [`MAX_DEPTH`], so only its start is shown.
    pub cut: bool,
    /// How many nodes the page was parsed into.
    pub nodes: usize,
}

/// The page `html` as the preview shows it, or `None` when the parser fails on it.
pub fn prepare(html: &str) -> Option<Page> {
    // The arena freezes at the bound and stands in a placeholder for later nodes; should the
    // tree builder trip over that, the page is not previewed rather than taking the app down.
    let parsed = std::panic::catch_unwind(|| {
        let mut parser = parse_document(Arena::new(), Default::default());
        let mut rest = html;
        while !rest.is_empty() && !parser.tokenizer.sink.sink.full {
            let mut end = rest.len().min(CHUNK);
            while !rest.is_char_boundary(end) {
                end += 1;
            }
            parser.process(StrTendril::from_slice(&rest[..end]));
            rest = &rest[end..];
        }
        parser.finish()
    });
    let arena = parsed.ok()?;
    Some(Page {
        markdown: Writer::new(&arena).write(),
        cut: arena.full,
        nodes: arena.nodes.len(),
    })
}

/// Where a previewed image comes from: a raster `data:` image decoded off the main thread under
/// the preview caps. Any other URL, or a refused image, loads as an error and shows nothing, so
/// nothing is fetched.
pub fn image_source(uri: &SharedUri) -> ImageSource {
    let uri = uri.clone();
    ImageSource::Custom(Arc::new(move |window, cx| {
        window.use_asset::<DataImage>(&uri, cx)
    }))
}

// Loads a `data:` image on gpui's background executor, cached by its URL.
enum DataImage {}

impl Asset for DataImage {
    type Source = SharedUri;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    // An async fn would capture the App borrow, and the trait wants a 'static future.
    #[allow(clippy::manual_async_fn)]
    fn load(uri: SharedUri, _: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        async move {
            load_data_image(&uri)
                .map_err(|refusal| ImageCacheError::Asset(format!("{refusal:?}").into()))
        }
    }
}

/// The longest `data:` image payload the preview reads, as written in the page.
pub const MAX_DATA_IMAGE_BYTES: usize = 16 * 1024 * 1024;

/// Decodes a raster `data:` image under the preview caps. A payload over
/// [`MAX_DATA_IMAGE_BYTES`] is refused before it is read; any other URL is undecodable.
pub fn load_data_image(uri: &str) -> Result<Arc<RenderImage>, Refusal> {
    let (kind, bytes) = data_image(uri)?;
    image_guard::decode_preview(kind, &bytes, &image_guard::PREVIEW_LIMITS)
}

/// The preview of `markdown`, scrolling in its own list. Raw HTML is off and links are inert,
/// so nothing in the page reaches the HTML parser or the system opener.
pub fn view(id: impl Into<ElementId>, markdown: SharedString) -> TextView {
    TextView::markdown(id, markdown)
        .markdown_extensions(MarkdownExtensions::default().raw_html(false))
        .scrollable(true)
        .image_source(image_source)
        .on_link_click(|_, _, _, _| {})
}

// The kind and bytes of a raster `data:<mime>[;base64],<data>` image. Any other URL, an SVG
// among them, is undecodable, and a payload over the cap is refused before it is decoded.
fn data_image(uri: &str) -> Result<(ImageKind, Vec<u8>), Refusal> {
    data_parts(uri).ok_or(Refusal::Undecodable)?
}

fn data_parts(uri: &str) -> Option<Result<(ImageKind, Vec<u8>), Refusal>> {
    let rest = uri
        .get(..5)?
        .eq_ignore_ascii_case("data:")
        .then(|| &uri[5..])?;
    let (header, data) = rest.split_once(',')?;
    let mime = header.split(';').next()?.trim().to_ascii_lowercase();
    let kind = match mime.as_str() {
        "image/png" => ImageKind::Png,
        "image/jpeg" | "image/jpg" => ImageKind::Jpeg,
        "image/gif" => ImageKind::Gif,
        "image/webp" => ImageKind::Webp,
        "image/bmp" => ImageKind::Bmp,
        _ => return None,
    };
    if data.len() > MAX_DATA_IMAGE_BYTES {
        return Some(Err(Refusal::Oversized));
    }
    let base64 = (header.split(';').skip(1)).any(|p| p.trim().eq_ignore_ascii_case("base64"));
    let bytes = if base64 {
        use base64::Engine as _;
        let packed: String = data.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        base64::engine::general_purpose::STANDARD
            .decode(packed)
            .ok()?
    } else {
        percent_encoding::percent_decode_str(data).collect()
    };
    Some(Ok((kind, bytes)))
}

// The parsed page: nodes in a flat list, linked by index, so neither building nor dropping it
// recurses however deep the page nests.
struct Arena {
    nodes: Vec<Node>,
    // Set at the node or depth bound; from then on the tree no longer changes.
    full: bool,
    // The name a placeholder answers to.
    span: QualName,
}

struct Node {
    parent: Option<usize>,
    children: Vec<usize>,
    kind: Kind,
}

enum Kind {
    Document,
    Element {
        name: QualName,
        attrs: Vec<Attribute>,
        integration: bool,
    },
    Text(String),
    Other,
}

const DOCUMENT: usize = 0;
// A detached element handed out for every node created past the bound.
const PLACEHOLDER: usize = 1;

impl Arena {
    fn new() -> Arena {
        let span = QualName::new(None, ns!(html), local_name!("span"));
        let placeholder = Kind::Element {
            name: span.clone(),
            attrs: Vec::new(),
            integration: false,
        };
        let node = |kind| Node {
            parent: None,
            children: Vec::new(),
            kind,
        };
        Arena {
            nodes: vec![node(Kind::Document), node(placeholder)],
            full: false,
            span,
        }
    }

    fn add(&mut self, kind: Kind) -> usize {
        if self.full || self.nodes.len() >= MAX_NODES {
            self.full = true;
            return PLACEHOLDER;
        }
        self.nodes.push(Node {
            parent: None,
            children: Vec::new(),
            kind,
        });
        self.nodes.len() - 1
    }

    fn detach(&mut self, node: usize) {
        if let Some(parent) = self.nodes[node].parent.take() {
            let siblings = &mut self.nodes[parent].children;
            if let Some(ix) = siblings.iter().rposition(|&n| n == node) {
                siblings.remove(ix);
            }
        }
    }

    // How far below the document `node` sits, counted up its parent links and capped just
    // past the bound, so a subtree moved before its new parent was attached is never stale.
    fn depth(&self, mut node: usize) -> usize {
        let mut depth = 0;
        while let Some(parent) = self.nodes[node].parent
            && depth <= MAX_DEPTH
        {
            node = parent;
            depth += 1;
        }
        depth
    }

    // Puts `child` into `parent` at `ix`, merging text into a text node just before it.
    fn insert(&mut self, parent: usize, ix: usize, child: NodeOrText<usize>) {
        match child {
            NodeOrText::AppendNode(node) => {
                self.detach(node);
                let ix = ix.min(self.nodes[parent].children.len());
                let depth = self.depth(parent) + 1;
                self.nodes[node].parent = Some(parent);
                self.nodes[parent].children.insert(ix, node);
                self.full |= depth > MAX_DEPTH;
            }
            NodeOrText::AppendText(text) => {
                let before = ix.checked_sub(1).map(|i| self.nodes[parent].children[i]);
                if let Some(before) = before
                    && let Kind::Text(existing) = &mut self.nodes[before].kind
                {
                    existing.push_str(&text);
                    return;
                }
                let node = self.add(Kind::Text(text.to_string()));
                if node != PLACEHOLDER {
                    self.nodes[node].parent = Some(parent);
                    self.nodes[parent].children.insert(ix, node);
                }
            }
        }
    }
}

impl TreeSink for Arena {
    type Handle = usize;
    type Output = Arena;

    fn finish(self) -> Arena {
        self
    }

    fn parse_error(&mut self, _: Cow<'static, str>) {}

    fn get_document(&mut self) -> usize {
        DOCUMENT
    }

    fn elem_name<'a>(&'a self, target: &'a usize) -> ExpandedName<'a> {
        match &self.nodes[*target].kind {
            Kind::Element { name, .. } => name.expanded(),
            _ => self.span.expanded(),
        }
    }

    fn create_element(
        &mut self,
        name: QualName,
        attrs: Vec<Attribute>,
        flags: ElementFlags,
    ) -> usize {
        self.add(Kind::Element {
            name,
            attrs,
            integration: flags.mathml_annotation_xml_integration_point,
        })
    }

    fn create_comment(&mut self, _: StrTendril) -> usize {
        self.add(Kind::Other)
    }

    fn create_pi(&mut self, _: StrTendril, _: StrTendril) -> usize {
        self.add(Kind::Other)
    }

    fn append(&mut self, parent: &usize, child: NodeOrText<usize>) {
        if !self.full {
            let end = self.nodes[*parent].children.len();
            self.insert(*parent, end, child);
        }
    }

    fn append_based_on_parent_node(
        &mut self,
        element: &usize,
        prev: &usize,
        child: NodeOrText<usize>,
    ) {
        if self.nodes[*element].parent.is_some() {
            self.append_before_sibling(element, child);
        } else {
            self.append(prev, child);
        }
    }

    fn append_doctype_to_document(&mut self, _: StrTendril, _: StrTendril, _: StrTendril) {}

    // Template contents are kept as the template's children; the preview leaves templates out.
    fn get_template_contents(&mut self, target: &usize) -> usize {
        *target
    }

    fn same_node(&self, x: &usize, y: &usize) -> bool {
        x == y
    }

    fn set_quirks_mode(&mut self, _: QuirksMode) {}

    fn append_before_sibling(&mut self, sibling: &usize, child: NodeOrText<usize>) {
        if self.full {
            return;
        }
        if let NodeOrText::AppendNode(node) = &child {
            self.detach(*node);
        }
        let Some(parent) = self.nodes[*sibling].parent else {
            return;
        };
        let siblings = &self.nodes[parent].children;
        let ix = siblings
            .iter()
            .rposition(|n| n == sibling)
            .unwrap_or(siblings.len());
        self.insert(parent, ix, child);
    }

    fn add_attrs_if_missing(&mut self, target: &usize, new: Vec<Attribute>) {
        if self.full {
            return;
        }
        if let Kind::Element { attrs, .. } = &mut self.nodes[*target].kind {
            for attr in new {
                if !attrs.iter().any(|a| a.name == attr.name) {
                    attrs.push(attr);
                }
            }
        }
    }

    fn remove_from_parent(&mut self, target: &usize) {
        if !self.full {
            self.detach(*target);
        }
    }

    fn reparent_children(&mut self, node: &usize, new_parent: &usize) {
        if self.full {
            return;
        }
        let children = std::mem::take(&mut self.nodes[*node].children);
        for &child in &children {
            self.nodes[child].parent = Some(*new_parent);
        }
        self.nodes[*new_parent].children.extend(children);
    }

    fn is_mathml_annotation_xml_integration_point(&self, handle: &usize) -> bool {
        matches!(
            self.nodes[*handle].kind,
            Kind::Element {
                integration: true,
                ..
            }
        )
    }
}

// A container a block line is prefixed by: a quote, or a list item whose marker shows on its
// first line and whose width indents the rest. `first` holds until its first block is written.
enum Container {
    Quote { first: bool },
    Item { marker: String, first: bool },
}

// Which line of a block a prefix is for.
#[derive(Clone, Copy, PartialEq)]
enum Line {
    Gap,
    First,
    Rest,
}

// What leaving an element undoes.
#[derive(Clone, Copy)]
enum Opened {
    Nothing,
    Container,
    List,
    Mark,
    Pre,
    Code,
    Table,
    Row,
    Cell,
    Heading(usize),
    Block,
}

enum Step {
    Enter(usize),
    Exit(Opened),
}

struct Table {
    rows: Vec<Vec<String>>,
}

// Writes the arena as Markdown in one walk with an explicit stack. Page text is collapsed as
// HTML collapses whitespace and has every ASCII punctuation escaped, so it makes no syntax;
// the only syntax is what the walk writes, and its nesting is capped.
struct Writer<'a> {
    arena: &'a Arena,
    out: String,
    line: String,
    space: bool,
    nest: Vec<Container>,
    lists: Vec<(bool, usize)>,
    mark: Option<&'static str>,
    marks: usize,
    mark_written: bool,
    pre: Option<String>,
    code: Option<String>,
    // A space pending when the code span began, written before it.
    code_space: bool,
    codes: usize,
    table: Option<Table>,
    tables: usize,
    cell: bool,
}

impl<'a> Writer<'a> {
    fn new(arena: &'a Arena) -> Writer<'a> {
        Writer {
            arena,
            out: String::new(),
            line: String::new(),
            space: false,
            nest: Vec::new(),
            lists: Vec::new(),
            mark: None,
            marks: 0,
            mark_written: false,
            pre: None,
            code: None,
            code_space: false,
            codes: 0,
            table: None,
            tables: 0,
            cell: false,
        }
    }

    fn write(mut self) -> String {
        let mut steps = vec![Step::Enter(DOCUMENT)];
        while let Some(step) = steps.pop() {
            match step {
                Step::Enter(node) => {
                    let Some(opened) = self.enter(node) else {
                        continue;
                    };
                    steps.push(Step::Exit(opened));
                    let children = &self.arena.nodes[node].children;
                    steps.extend(children.iter().rev().map(|&c| Step::Enter(c)));
                }
                Step::Exit(opened) => self.exit(opened),
            }
        }
        self.flush();
        self.out
    }

    // Starts `node`; `None` leaves out the node and everything inside it.
    fn enter(&mut self, node: usize) -> Option<Opened> {
        let (name, attrs) = match &self.arena.nodes[node].kind {
            Kind::Document => return Some(Opened::Nothing),
            Kind::Text(text) => {
                self.text(text);
                return None;
            }
            Kind::Other => return None,
            Kind::Element { name, attrs, .. } => (name, attrs),
        };
        if name.ns != ns!(html) {
            return None;
        }
        let tag = &name.local;
        let opened = match &**tag {
            "head" | "script" | "style" | "title" | "template" | "noscript" | "iframe"
            | "object" | "embed" | "canvas" | "video" | "audio" | "select" | "frameset"
            | "noembed" | "noframes" => return None,
            _ if self.pre.is_some() => {
                if tag == &local_name!("br") {
                    self.text("\n");
                }
                Opened::Nothing
            }
            "b" | "strong" | "i" | "em" => {
                self.marks += 1;
                if self.marks == 1 {
                    let strong = matches!(tag.as_ref(), "b" | "strong");
                    self.mark = Some(if strong { "**" } else { "*" });
                    self.mark_written = false;
                }
                Opened::Mark
            }
            "code" | "kbd" | "samp" | "tt" => {
                self.codes += 1;
                if self.codes == 1 {
                    self.code = Some(String::new());
                    self.code_space = std::mem::take(&mut self.space);
                }
                Opened::Code
            }
            "img" => {
                self.image(attrs);
                Opened::Nothing
            }
            "br" | "hr" if self.cell => {
                self.space = true;
                Opened::Nothing
            }
            "br" => {
                self.flush();
                Opened::Nothing
            }
            "hr" => {
                self.flush();
                self.block("* * *");
                Opened::Nothing
            }
            "table" if self.tables == 0 && !self.cell => {
                self.flush();
                self.tables = 1;
                self.table = Some(Table { rows: Vec::new() });
                Opened::Table
            }
            "table" => {
                self.tables += 1;
                self.space = true;
                Opened::Table
            }
            "tr" if self.tables == 1 => {
                if let Some(table) = &mut self.table {
                    table.rows.push(Vec::new());
                }
                Opened::Row
            }
            "td" | "th" if self.tables == 1 && !self.cell => {
                self.cell = true;
                self.line.clear();
                self.space = false;
                Opened::Cell
            }
            "td" | "th" | "tr" => {
                self.space = true;
                Opened::Nothing
            }
            _ if self.table.is_some() && !self.cell => Opened::Nothing,
            "pre" if !self.cell => {
                self.flush();
                self.pre = Some(String::new());
                Opened::Pre
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" if !self.cell => {
                self.flush();
                Opened::Heading(usize::from(tag.as_bytes()[1] - b'0'))
            }
            "ul" | "ol" => {
                self.flush();
                self.lists.push((tag == &local_name!("ol"), 0));
                Opened::List
            }
            "li" | "blockquote" if !self.cell && self.nest.len() < MAX_NEST => {
                self.flush();
                let container = if tag == &local_name!("li") {
                    let marker = match self.lists.last_mut() {
                        Some((true, n)) => {
                            *n += 1;
                            format!("{n}. ")
                        }
                        _ => "- ".to_owned(),
                    };
                    Container::Item {
                        marker,
                        first: true,
                    }
                } else {
                    Container::Quote { first: true }
                };
                self.nest.push(container);
                Opened::Container
            }
            _ if is_block(tag) => {
                self.flush();
                Opened::Block
            }
            _ => Opened::Nothing,
        };
        Some(opened)
    }

    fn exit(&mut self, opened: Opened) {
        match opened {
            Opened::Nothing => {}
            Opened::Block => self.flush(),
            Opened::Mark => {
                self.marks -= 1;
                if self.marks == 0 {
                    self.close_mark();
                    self.mark = None;
                }
            }
            Opened::Code => {
                self.codes -= 1;
                if self.codes == 0
                    && let Some(code) = self.code.take()
                {
                    let after = std::mem::replace(&mut self.space, self.code_space);
                    self.code_span(code.trim());
                    self.space |= after;
                }
            }
            Opened::Pre => {
                if let Some(text) = self.pre.take() {
                    let text = text.trim_end_matches('\n');
                    let fence = "`".repeat(longest_run(text, '`').max(2) + 1);
                    self.block(&format!("{fence}\n{text}\n{fence}"));
                }
            }
            Opened::Heading(level) => {
                self.close_mark();
                let text = std::mem::take(&mut self.line);
                self.space = false;
                if !text.is_empty() {
                    self.block(&format!("{} {text}", "#".repeat(level)));
                }
            }
            Opened::List => {
                self.flush();
                self.lists.pop();
            }
            Opened::Container => {
                self.flush();
                self.nest.pop();
            }
            Opened::Cell => {
                self.close_mark();
                self.cell = false;
                let text = std::mem::take(&mut self.line);
                self.space = false;
                if let Some(row) = self.table.as_mut().and_then(|t| t.rows.last_mut()) {
                    row.push(text);
                }
            }
            Opened::Row => {}
            Opened::Table => {
                self.tables -= 1;
                if self.tables == 0
                    && let Some(table) = self.table.take()
                {
                    self.table_block(table);
                }
            }
        }
    }

    // Adds page text: raw inside a code block or span, otherwise collapsed and escaped.
    fn text(&mut self, text: &str) {
        let text = &plain(text);
        if let Some(pre) = &mut self.pre {
            pre.push_str(text);
            return;
        }
        if self.table.is_some() && !self.cell {
            return;
        }
        for c in text.chars() {
            if matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c') {
                self.space = true;
                continue;
            }
            if let Some(code) = &mut self.code {
                if self.space && !code.is_empty() {
                    code.push(' ');
                }
                self.space = false;
                code.push(c);
                continue;
            }
            self.word_start();
            if c.is_ascii_punctuation() {
                self.line.push('\\');
            }
            self.line.push(c);
        }
    }

    // Writes what goes before the next visible character: a collapsed space, then an
    // emphasis delimiter that is open but not yet written.
    fn word_start(&mut self) {
        if self.space && !self.line.is_empty() {
            self.line.push(' ');
        }
        self.space = false;
        if let Some(mark) = self.mark
            && !self.mark_written
        {
            self.line.push_str(mark);
            self.mark_written = true;
        }
    }

    // Closes the open delimiter, if one was written; whitespace after it stays pending.
    fn close_mark(&mut self) {
        if let Some(mark) = self.mark
            && self.mark_written
        {
            self.line.push_str(mark);
            self.mark_written = false;
        }
    }

    fn code_span(&mut self, code: &str) {
        if code.is_empty() || self.table.is_some() && !self.cell {
            return;
        }
        let code = if self.cell {
            code.replace('|', "\\|")
        } else {
            code.to_owned()
        };
        let fence = "`".repeat(longest_run(&code, '`') + 1);
        let pad = if code.starts_with(['`', ' ']) || code.ends_with(['`', ' ']) {
            " "
        } else {
            ""
        };
        self.word_start();
        self.line
            .push_str(&format!("{fence}{pad}{code}{pad}{fence}"));
    }

    // A raster `data:` image as an image, any other as its alt text.
    fn image(&mut self, attrs: &[Attribute]) {
        let attr = |name: &str| {
            attrs
                .iter()
                .find(|a| &*a.name.local == name)
                .map(|a| a.value.to_string())
        };
        let alt = attr("alt").unwrap_or_default();
        let src: Option<String> =
            attr("src").map(|s| s.chars().filter(|c| !c.is_ascii_whitespace()).collect());
        match src.filter(|s| data_image(s).is_ok() && self.code.is_none()) {
            Some(src) if self.table.is_none() || self.cell => {
                self.word_start();
                self.line.push_str("![");
                for c in plain(&alt).chars().filter(|c| !c.is_control()) {
                    if c.is_ascii_punctuation() {
                        self.line.push('\\');
                    }
                    self.line.push(c);
                }
                self.line.push_str("](");
                self.line.push_str(&destination(&src));
                self.line.push(')');
            }
            _ => self.text(&alt),
        }
    }

    // Ends the paragraph being written; inside a table cell it only leaves a space.
    fn flush(&mut self) {
        if self.cell {
            self.space = true;
            return;
        }
        self.close_mark();
        self.space = false;
        if !self.line.is_empty() {
            let line = std::mem::take(&mut self.line);
            self.block(&line);
        }
    }

    fn table_block(&mut self, table: Table) {
        let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        if columns == 0 {
            return;
        }
        let row = |cells: &[String]| {
            let cells = (0..columns).map(|i| cells.get(i).map_or("", String::as_str));
            format!("| {} |", cells.collect::<Vec<_>>().join(" | "))
        };
        let mut lines = vec![row(&table.rows[0]), row(&vec!["---".to_owned(); columns])];
        lines.extend(table.rows[1..].iter().map(|r| row(r)));
        self.block(&lines.join("\n"));
    }

    // Writes one block, a blank line after the last, each line behind its containers' prefix.
    fn block(&mut self, text: &str) {
        if !self.out.is_empty() {
            let gap = self.prefix(Line::Gap);
            self.out.push_str(gap.trim_end());
            self.out.push('\n');
        }
        for (ix, line) in text.split('\n').enumerate() {
            let prefix = self.prefix(if ix == 0 { Line::First } else { Line::Rest });
            self.out.push_str(&prefix);
            self.out.push_str(line);
            self.out.push('\n');
        }
        for container in &mut self.nest {
            let (Container::Quote { first } | Container::Item { first, .. }) = container;
            *first = false;
        }
    }

    // The prefix of a block's first line, its later lines, or the gap before it; a quote
    // that has written nothing yet leaves the gap outside it.
    fn prefix(&self, line: Line) -> String {
        let mut prefix = String::new();
        for container in &self.nest {
            match container {
                Container::Quote { first: true } if line == Line::Gap => {}
                Container::Quote { .. } => prefix.push_str("> "),
                Container::Item {
                    marker,
                    first: true,
                } if line == Line::First => prefix.push_str(marker),
                Container::Item { marker, .. } => prefix.push_str(&" ".repeat(marker.len())),
            }
        }
        prefix
    }
}

fn is_block(tag: &LocalName) -> bool {
    matches!(
        tag.as_ref(),
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "body"
            | "caption"
            | "center"
            | "dd"
            | "details"
            | "dialog"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "header"
            | "hgroup"
            | "html"
            | "legend"
            | "li"
            | "main"
            | "menu"
            | "nav"
            | "p"
            | "search"
            | "section"
            | "summary"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "pre"
    )
}

// Page text with every line ending Markdown knows as `\n` and every other control or line
// separator as a space, so no page text starts a line outside its containers' prefix.
fn plain(text: &str) -> String {
    let text = text.replace("\r\n", "\n");
    let plain = |c: char| match c {
        '\n' | '\t' => c,
        '\r' => '\n',
        _ if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') => ' ',
        _ => c,
    };
    text.chars().map(plain).collect()
}

// The longest run of `c` in `text`.
fn longest_run(text: &str, c: char) -> usize {
    let (mut longest, mut run) = (0, 0);
    for ch in text.chars() {
        run = if ch == c { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    longest
}

// `url` as a Markdown destination: every byte outside the URL characters, and the ones that
// end or open a destination, percent-encoded.
fn destination(url: &str) -> String {
    let mut out = String::with_capacity(url.len());
    for b in url.bytes() {
        let plain = b.is_ascii_alphanumeric() || b"-._~:/?#[]@!$&'*+,;=%".contains(&b);
        if plain {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // The adoption agency moves an element's children under a new element before attaching
    // it; nesting under a moved child must still count its depth from the document.
    #[test]
    fn a_child_moved_before_its_new_parent_is_attached_keeps_its_depth() {
        let mut arena = Arena::new();
        let div = QualName::new(None, ns!(html), local_name!("div"));
        let nest = |arena: &mut Arena, parent: usize| {
            let node = arena.create_element(div.clone(), Vec::new(), ElementFlags::default());
            arena.append(&parent, NodeOrText::AppendNode(node));
            node
        };
        let mut top = DOCUMENT;
        for _ in 0..MAX_DEPTH - 10 {
            top = nest(&mut arena, top);
        }
        let mut deep = nest(&mut arena, top);
        let moved_to = arena.create_element(div.clone(), Vec::new(), ElementFlags::default());
        arena.reparent_children(&top, &moved_to);
        arena.append(&top, NodeOrText::AppendNode(moved_to));
        for _ in 0..20 {
            deep = nest(&mut arena, deep);
        }
        assert!(arena.full);
    }
}
