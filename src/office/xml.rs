//! Small helpers over quick-xml's reader.

use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::reader::Reader;

pub fn reader(data: &[u8]) -> Reader<&[u8]> {
    let mut reader = Reader::from_reader(data);
    let config = reader.config_mut();
    config.trim_text(false);
    config.check_end_names = false;
    reader
}

/// The element name without its namespace prefix.
pub fn local(e: &BytesStart) -> Vec<u8> {
    e.local_name().as_ref().to_vec()
}

/// The value of the attribute whose local name is `name` (`w:val` is `val`).
pub fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        if a.key.local_name().as_ref() == name.as_bytes() {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        } else {
            None
        }
    })
}

/// The value of a relationship id attribute (`r:id`, `r:embed`), whatever the prefix:
/// the unprefixed `id` of the same element is a different thing.
pub fn rel_id(e: &BytesStart) -> Option<String> {
    e.attributes().flatten().find_map(|a| {
        let key = a.key;
        if key.prefix().is_some() && key.local_name().as_ref() == b"id" {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        } else {
            None
        }
    })
}

/// The text of an entity reference: `&amp;` and friends, or a numeric character reference.
pub fn entity(r: &BytesRef) -> Option<String> {
    if let Ok(Some(c)) = r.resolve_char_ref() {
        return Some(c.to_string());
    }
    let name = r.decode().ok()?;
    Some(
        match name.as_ref() {
            "amp" => "&",
            "lt" => "<",
            "gt" => ">",
            "quot" => "\"",
            "apos" => "'",
            _ => return None,
        }
        .to_string(),
    )
}

/// The text of a text, CDATA or entity event.
pub fn text_of(event: &Event) -> Option<String> {
    match event {
        Event::Text(t) => t.decode().ok().map(|s| s.into_owned()),
        Event::CData(c) => c.decode().ok().map(|s| s.into_owned()),
        Event::GeneralRef(r) => entity(r),
        _ => None,
    }
}

/// The text of the first element called `wanted`: for small metadata parts.
pub fn element_text(data: &[u8], wanted: &str) -> Option<String> {
    let mut reader = reader(data);
    let mut inside = false;
    let mut text = String::new();
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) if e.local_name().as_ref() == wanted.as_bytes() => inside = true,
            Event::End(e) if e.local_name().as_ref() == wanted.as_bytes() => return Some(text),
            Event::Eof => return None,
            ev if inside => {
                if let Some(t) = text_of(&ev) {
                    text.push_str(&t);
                }
            }
            _ => {}
        }
    }
}
