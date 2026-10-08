//! `stringtable.xml`: `<Key ID="...">` (or legacy `<Text ID="...">`) elements anywhere in the
//! tree, each with one child element per language.

use roxmltree::{Document, Node};

use crate::{Entry, Error, Result, Stringtable};

pub fn parse(text: &str) -> Result<Stringtable> {
    let doc = Document::parse(text).map_err(|e| Error::Xml(e.to_string()))?;
    let entries = doc
        .descendants()
        .filter(|n| n.is_element() && matches!(n.tag_name().name(), "Key" | "Text"))
        .filter_map(|key| {
            let id = key.attribute("ID")?;
            Some(Entry {
                key: id.to_string(),
                translations: key
                    .children()
                    .filter(Node::is_element)
                    .map(|lang| (lang.tag_name().name().to_string(), text_of(lang)))
                    .collect(),
            })
        })
        .collect();
    Ok(Stringtable { entries })
}

/// All character data directly inside `node` (text and CDATA), concatenated.
fn text_of(node: Node) -> String {
    node.children()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect()
}
