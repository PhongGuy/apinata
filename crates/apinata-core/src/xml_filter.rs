//! XPath 1.0 filtering of XML response bodies, behind an apinata-owned API.

use uppsala::{
    Document, Element, NodeId, NodeKind, Parser, QName, XPathEvaluator, XPathValue, XmlError,
};

use crate::json_filter::FilterError;

/// Node visits one evaluation may spend.
const XPATH_NODE_VISITS: usize = 10_000_000;

/// Runs the XPath 1.0 `expression` over the XML `body`. A node set comes back one node per line
/// in document order (elements as XML, other nodes as their string values); a scalar comes back
/// as one line holding its XPath string value. Every line ends in a newline, so no match is `""`.
/// Prefixes declared on the root element are bound for the expression.
pub fn filter_xml(body: &str, expression: &str) -> Result<String, FilterError> {
    let filter = XmlFilter::compile(expression)?;
    filter.run(&ParsedXml::parse(body)?, || false)
}

/// An XML body parsed once, so several filters can run over it without parsing it again.
#[derive(Debug)]
#[non_exhaustive]
pub struct ParsedXml {
    doc: Document<'static>,
}

impl ParsedXml {
    /// Parses `body`, skipping a leading byte order mark; fails with [`FilterError::Xml`].
    pub fn parse(body: &str) -> Result<Self, FilterError> {
        let mut doc = Parser::new()
            .parse(body.strip_prefix('\u{feff}').unwrap_or(body))
            .map_err(|e| FilterError::Xml {
                message: e.to_string(),
            })?;
        doc.prepare_xpath();
        Ok(ParsedXml {
            doc: doc.into_static(),
        })
    }
}

/// An XPath filter, as [`filter_xml`] reads the expression.
#[derive(Debug)]
#[non_exhaustive]
pub struct XmlFilter {
    expression: String,
}

impl XmlFilter {
    /// Takes `expression`; fails with [`FilterError::Expression`] for an empty one. The engine
    /// parses the expression only when it runs, so other syntax errors come from [`Self::run`].
    pub fn compile(expression: &str) -> Result<Self, FilterError> {
        let typed = expression.trim();
        if typed.is_empty() {
            return Err(FilterError::Expression {
                message: "the filter is empty".to_owned(),
                position: 0,
            });
        }
        Ok(XmlFilter {
            expression: typed.to_owned(),
        })
    }

    /// Runs the filter over `body`, printed as [`filter_xml`] prints it. `cancelled` is asked
    /// before the evaluation and before each match written; once it says yes the run stops with
    /// [`FilterError::Cancelled`]. The evaluation itself cannot stop midway; the work cap bounds it.
    pub fn run(
        &self,
        body: &ParsedXml,
        cancelled: impl Fn() -> bool,
    ) -> Result<String, FilterError> {
        if cancelled() {
            return Err(FilterError::Cancelled);
        }
        let doc = &body.doc;
        // Bind the root's prefixes so `x:id` works as written; the default namespace stays
        // unbound, as XPath 1.0 has no way to name it.
        let mut evaluator = XPathEvaluator::new();
        if let Some(element) = doc.document_element().and_then(|id| doc.element(id)) {
            for (prefix, uri) in &element.namespace_declarations {
                if !prefix.is_empty() {
                    evaluator.add_namespace(prefix.as_ref(), uri.as_ref());
                }
            }
        }
        let value = evaluator
            .with_max_node_visits(XPATH_NODE_VISITS)
            .evaluate(doc, doc.root(), &self.expression)
            .map_err(evaluation_error)?;

        // Every match is a line ending in a newline, so an empty match still prints a line and
        // only an empty node set prints nothing.
        let mut out = String::new();
        match &value {
            XPathValue::NodeSet(nodes) => {
                for &id in nodes {
                    if cancelled() {
                        return Err(FilterError::Cancelled);
                    }
                    out.push_str(&node_text(doc, id));
                    out.push('\n');
                }
            }
            scalar => {
                out.push_str(&scalar.to_string_value(doc));
                out.push('\n');
            }
        }
        Ok(out)
    }
}

/// Sorts an evaluation failure. The engine gives a spent budget no error kind of its own, so it is
/// known by the exact message it builds from this cap, which no echo of expression text can equal.
fn evaluation_error(error: XmlError) -> FilterError {
    match error {
        XmlError::XPath(e) if e.message == budget_message() => FilterError::Limit,
        other => FilterError::Invalid {
            message: other.to_string(),
        },
    }
}

fn budget_message() -> String {
    format!("XPath evaluation exceeded maximum node visit budget of {XPATH_NODE_VISITS}")
}

/// One matched node as shown: an element (or the root, standing for its document element) as XML,
/// anything else as its XPath string value.
fn node_text(doc: &Document<'_>, id: NodeId) -> String {
    match doc.node_kind(id) {
        Some(NodeKind::Element(_)) => element_xml(doc, id),
        Some(NodeKind::Document) => doc
            .document_element()
            .map(|root| element_xml(doc, root))
            .unwrap_or_default(),
        _ => XPathValue::NodeSet(vec![id]).to_string_value(doc),
    }
}

/// A matched element as standalone XML: it declares the ancestor namespaces its subtree uses,
/// so the fragment reads the same out of context, and everything inside is written as in the source.
fn element_xml(doc: &Document<'_>, id: NodeId) -> String {
    // Ancestor declarations from the root down; a nearer one replaces a farther one in place,
    // keeping the order a prefix first came into scope.
    let mut ancestors = Vec::new();
    let mut at = doc.parent(id);
    while let Some(parent) = at {
        ancestors.push(parent);
        at = doc.parent(parent);
    }
    let mut in_scope: Vec<(&str, &str)> = Vec::new();
    for element in ancestors.iter().rev().filter_map(|&a| doc.element(a)) {
        for (prefix, uri) in &element.namespace_declarations {
            match in_scope.iter_mut().find(|(p, _)| *p == prefix.as_ref()) {
                Some(entry) => entry.1 = uri.as_ref(),
                None => in_scope.push((prefix.as_ref(), uri.as_ref())),
            }
        }
    }

    let mut used = Vec::new();
    collect_outside_uses(doc, id, &mut Vec::new(), &mut used);
    let mut inherited: Vec<(&str, &str)> = in_scope
        .into_iter()
        .filter(|(prefix, uri)| !uri.is_empty() && used.contains(prefix))
        .collect();
    inherited.sort_by_key(|(prefix, _)| !prefix.is_empty());

    let mut out = String::new();
    write_element(doc, id, &inherited, &mut out);
    out
}

/// Gathers the prefixes (`""` for the default namespace) that names in the subtree at `id` use
/// without a declaration inside the subtree to resolve them.
fn collect_outside_uses<'d>(
    doc: &'d Document<'_>,
    id: NodeId,
    declared: &mut Vec<&'d str>,
    used: &mut Vec<&'d str>,
) {
    let Some(element) = doc.element(id) else {
        return;
    };
    let depth = declared.len();
    declared.extend(
        element
            .namespace_declarations
            .iter()
            .map(|(p, _)| p.as_ref()),
    );
    let element_prefix = match (&element.name.prefix, &element.name.namespace_uri) {
        (Some(prefix), _) => Some(prefix.as_ref()),
        (None, Some(_)) => Some(""),
        (None, None) => None,
    };
    let attribute_prefixes = element
        .attributes
        .iter()
        .filter_map(|a| a.name.prefix.as_deref());
    for prefix in element_prefix.into_iter().chain(attribute_prefixes) {
        if !declared.contains(&prefix) && !used.contains(&prefix) {
            used.push(prefix);
        }
    }
    for child in doc.children_iter(id) {
        collect_outside_uses(doc, child, declared, used);
    }
    declared.truncate(depth);
}

/// Writes one node in source form: `extra` declarations go first on this element only, then its
/// own declarations and attributes; an element with no children closes itself.
fn write_element(doc: &Document<'_>, id: NodeId, extra: &[(&str, &str)], out: &mut String) {
    match doc.node_kind(id) {
        Some(NodeKind::Element(element)) => {
            let Element {
                name,
                attributes,
                namespace_declarations,
            } = element;
            out.push('<');
            write_name(name, out);
            let own = namespace_declarations
                .iter()
                .map(|(p, u)| (p.as_ref(), u.as_ref()));
            for (prefix, uri) in extra.iter().copied().chain(own) {
                out.push_str(if prefix.is_empty() {
                    " xmlns"
                } else {
                    " xmlns:"
                });
                out.push_str(prefix);
                out.push_str("=\"");
                escape(uri, true, out);
                out.push('"');
            }
            for attribute in attributes {
                out.push(' ');
                write_name(&attribute.name, out);
                out.push_str("=\"");
                escape(&attribute.value, true, out);
                out.push('"');
            }
            if doc.children_count(id) == 0 {
                out.push_str("/>");
                return;
            }
            out.push('>');
            for child in doc.children_iter(id) {
                write_element(doc, child, &[], out);
            }
            out.push_str("</");
            write_name(name, out);
            out.push('>');
        }
        Some(NodeKind::Text(text)) => escape(text, false, out),
        Some(NodeKind::CData(text)) => {
            out.push_str("<![CDATA[");
            out.push_str(text);
            out.push_str("]]>");
        }
        Some(NodeKind::Comment(text)) => {
            out.push_str("<!--");
            out.push_str(text);
            out.push_str("-->");
        }
        Some(NodeKind::ProcessingInstruction(pi)) => {
            out.push_str("<?");
            out.push_str(&pi.target);
            if let Some(data) = &pi.data {
                out.push(' ');
                out.push_str(data);
            }
            out.push_str("?>");
        }
        _ => {}
    }
}

fn write_name(name: &QName<'_>, out: &mut String) {
    if let Some(prefix) = &name.prefix {
        out.push_str(prefix);
        out.push(':');
    }
    out.push_str(&name.local_name);
}

/// Escapes markup characters; an attribute value also escapes the double quote it is wrapped in.
/// Whitespace a parser would normalise (CR anywhere; tab and LF in attributes) is written as a
/// character reference so a re-parse yields the same value.
fn escape(text: &str, attribute: bool, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            '\r' => out.push_str("&#13;"),
            '\t' if attribute => out.push_str("&#9;"),
            '\n' if attribute => out.push_str("&#10;"),
            c => out.push(c),
        }
    }
}
