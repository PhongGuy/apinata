use apinata_app::body_text::*;
use std::time::Duration;

fn ct(v: &str) -> Vec<(String, String)> {
    vec![("Content-Type".into(), v.into())]
}

#[test]
fn kinds_follow_the_content_type_and_the_bytes() {
    assert_eq!(body_kind(&ct("application/json"), b""), BodyKind::Empty);
    assert_eq!(
        body_kind(&ct("application/json; charset=utf-8"), b"{}"),
        BodyKind::Json
    );
    assert_eq!(
        body_kind(&ct("application/problem+json"), b"{}"),
        BodyKind::Json
    );
    assert_eq!(body_kind(&ct("text/html"), b"<p>"), BodyKind::Html);
    assert_eq!(body_kind(&ct("text/plain"), b"hi"), BodyKind::Text);
    assert_eq!(body_kind(&[], b"hi"), BodyKind::Text);
    assert_eq!(
        body_kind(&ct("application/octet-stream"), &[0xff, 0xfe, 0x00]),
        BodyKind::Binary
    );
    assert_eq!(
        body_kind(&ct("text/plain; charset=latin1"), &[0xe9, 0x74, 0xe9]),
        BodyKind::Text
    );
}

#[test]
fn an_incomplete_last_character_is_still_text() {
    assert_eq!(body_kind(&[], &[b'a', 0xc3]), BodyKind::Text);
    assert_eq!(body_kind(&[], &[0xc3, b'a']), BodyKind::Binary);
}

#[test]
fn reindent_matches_json_stringify_with_two_spaces() {
    let got = reindent_json("{\"a\":1,\"b\":[1,2],\"c\":{},\"d\":[]}").unwrap();
    assert_eq!(
        got,
        "{\n  \"a\": 1,\n  \"b\": [\n    1,\n    2\n  ],\n  \"c\": {},\n  \"d\": []\n}"
    );
    assert_eq!(
        reindent_json(" { \"a\" :  [ ] } ").unwrap(),
        "{\n  \"a\": []\n}"
    );
    assert_eq!(reindent_json("\"x\"").unwrap(), "\"x\"");
    assert_eq!(reindent_json("42").unwrap(), "42");
}

#[test]
fn reindent_keeps_tokens_as_sent() {
    let got = reindent_json("[1.10,1e5,12345678901234567890]").unwrap();
    assert_eq!(got, "[\n  1.10,\n  1e5,\n  12345678901234567890\n]");
    assert_eq!(
        reindent_json("{\"a\":1,\"a\":2}").unwrap(),
        "{\n  \"a\": 1,\n  \"a\": 2\n}"
    );
    assert_eq!(
        reindent_json("{\"s\":\"a,{b}\\\"c: [\"}").unwrap(),
        "{\n  \"s\": \"a,{b}\\\"c: [\"\n}"
    );
}

#[test]
fn invalid_json_is_not_reindented() {
    assert_eq!(reindent_json("{\"a\":}"), None);
    assert_eq!(reindent_json("not json"), None);
    assert_eq!(reindent_json("{\"a\":1"), None);
}

#[test]
fn prepare_picks_language_and_view() {
    let json = "{\"a\":1}";
    let pretty = Prepared::Editor {
        text: "{\n  \"a\": 1\n}".into(),
        language: "json",
    };
    assert_eq!(prepare(BodyKind::Json, json, 7, true), pretty);
    assert_eq!(
        prepare(BodyKind::Json, json, 7, false),
        Prepared::Editor {
            text: json.into(),
            language: "text"
        }
    );
    assert_eq!(
        prepare(BodyKind::Json, "{oops", 5, true),
        Prepared::Editor {
            text: "{oops".into(),
            language: "json"
        }
    );
    assert_eq!(
        prepare(BodyKind::Html, "<p>", 3, true),
        Prepared::Editor {
            text: "<p>".into(),
            language: "html"
        }
    );
    assert_eq!(
        prepare(BodyKind::Binary, "", 9, true),
        Prepared::Binary { size: 9 }
    );
    assert_eq!(prepare(BodyKind::Empty, "", 0, true), Prepared::Empty);
}

#[test]
fn the_editor_limits_switch_to_rows() {
    let at = vec!["x"; EDITOR_MAX_LINES].join("\n");
    assert!(matches!(
        prepare(BodyKind::Text, &at, 0, true),
        Prepared::Editor { .. }
    ));
    let over = vec!["x"; EDITOR_MAX_LINES + 1].join("\n");
    assert!(
        matches!(prepare(BodyKind::Text, &over, 0, true), Prepared::Lines(r) if r.len() == EDITOR_MAX_LINES + 1)
    );
    let wide = "y".repeat(EDITOR_MAX_BYTES + 1);
    assert!(
        matches!(prepare(BodyKind::Text, &wide, 0, true), Prepared::Lines(r) if r.len() == wide.len().div_ceil(ROW_WIDTH))
    );
}

#[test]
fn rows_split_lines_and_chunk_on_char_boundaries() {
    let r = rows("ab\r\ncd\n", 10);
    assert_eq!(
        r.iter().map(|s| s.as_ref()).collect::<Vec<_>>(),
        ["ab", "cd", ""]
    );
    let r = rows("aéb", 2);
    assert_eq!(
        r.iter().map(|s| s.as_ref()).collect::<Vec<_>>(),
        ["a", "é", "b"]
    );
}

#[test]
fn a_size_pair_shares_one_unit() {
    use apinata_app::body_text::format_size_pair;
    // A compressed body read to the cut is far smaller than its decoded size.
    assert_eq!(
        format_size_pair(1_153_434, 307_200),
        ("1.1 MB".to_string(), "0.3 MB".to_string())
    );
    assert_eq!(
        format_size_pair(307_200, 1_153_434),
        ("0.3 MB".to_string(), "1.1 MB".to_string())
    );
    assert_eq!(
        format_size_pair(2048, 512),
        ("2.0 KB".to_string(), "0.5 KB".to_string())
    );
    assert_eq!(
        format_size_pair(900, 5),
        ("900 B".to_string(), "5 B".to_string())
    );
    // Equal sizes still match the single-size format.
    assert_eq!(format_size_pair(10_485_760, 10_485_760).0, "10.0 MB");
}

#[test]
fn an_untruncated_size_keeps_its_own_unit() {
    use apinata_app::body_text::size_labels;
    assert_eq!(
        size_labels(900, 5_000_000, false),
        ("900 B".to_string(), None)
    );
    assert_eq!(
        size_labels(1_153_434, 307_200, true),
        ("1.1 MB".to_string(), Some("0.3 MB".to_string()))
    );
}

#[test]
fn sizes_and_times_read_naturally() {
    assert_eq!(format_size(512), "512 B");
    assert_eq!(format_size(1536), "1.5 KB");
    assert_eq!(format_size(10_485_760), "10.0 MB");
    assert_eq!(format_time(Duration::from_millis(250)), "250 ms");
    assert_eq!(format_time(Duration::from_millis(1250)), "1.25 s");
}

#[test]
fn sub_millisecond_times_read_as_under_one() {
    assert_eq!(format_time(Duration::ZERO), "0 ms");
    assert_eq!(format_time(Duration::from_micros(400)), "<1 ms");
    assert_eq!(format_time(Duration::from_micros(1500)), "1 ms");
}

#[test]
fn xml_content_types_are_xml_and_xhtml_stays_html() {
    assert_eq!(body_kind(&ct("application/xml"), b"<a/>"), BodyKind::Xml);
    assert_eq!(
        body_kind(&ct("text/xml; charset=utf-8"), b"<a/>"),
        BodyKind::Xml
    );
    assert_eq!(
        body_kind(&ct("application/soap+xml"), b"<a/>"),
        BodyKind::Xml
    );
    assert_eq!(
        body_kind(&ct("application/atom+xml"), b"<a/>"),
        BodyKind::Xml
    );
    assert_eq!(
        body_kind(&ct("application/xhtml+xml"), b"<a/>"),
        BodyKind::Html
    );
}

#[test]
fn reindent_xml_nests_elements_by_two_spaces_and_keeps_the_rest() {
    assert_eq!(
        reindent_xml("<?xml version=\"1.0\"?><root><a x=\"1\">hi</a><b/><!-- c --></root>")
            .unwrap(),
        "<?xml version=\"1.0\"?>\n<root>\n  <a x=\"1\">hi</a>\n  <b/>\n  <!-- c -->\n</root>"
    );
    assert_eq!(
        reindent_xml("<r>\n   <a>one</a>\n\n  <b><![CDATA[x<y]]></b></r>").unwrap(),
        "<r>\n  <a>one</a>\n  <b><![CDATA[x<y]]></b>\n</r>"
    );
    assert_eq!(
        reindent_xml("<?xml version=\"1.0\"?>\n<!DOCTYPE note><note>&#169;</note>").unwrap(),
        "<?xml version=\"1.0\"?>\n<!DOCTYPE note>\n<note>&#169;</note>"
    );
}

#[test]
fn reindent_xml_keeps_each_text_run_whole() {
    assert_eq!(
        reindent_xml("<r><t>Tom &amp; Jerry</t><u>&lt;x&gt;</u></r>").unwrap(),
        "<r>\n  <t>Tom &amp; Jerry</t>\n  <u>&lt;x&gt;</u>\n</r>"
    );
    assert_eq!(
        reindent_xml("<a>hello <b>x</b> tail</a>").unwrap(),
        "<a>hello <b>x</b> tail</a>"
    );
}

#[test]
fn reindent_xml_puts_markup_after_an_element_on_its_own_line() {
    assert_eq!(
        reindent_xml("<s><a>1</a><![CDATA[x]]></s>").unwrap(),
        "<s>\n  <a>1</a>\n  <![CDATA[x]]>\n</s>"
    );
    assert_eq!(
        reindent_xml("<s><a/>\n <![CDATA[x]]><![CDATA[y]]><b>2</b></s>").unwrap(),
        "<s>\n  <a/>\n  <![CDATA[x]]>\n  <![CDATA[y]]>\n  <b>2</b>\n</s>"
    );
    assert_eq!(
        reindent_xml("<s><a>1</a><!-- c --><?p q?></s>").unwrap(),
        "<s>\n  <a>1</a>\n  <!-- c -->\n  <?p q?>\n</s>"
    );
}

#[test]
fn reindent_xml_refuses_mismatched_tags() {
    assert_eq!(reindent_xml("<a><b></a>"), None);
}

#[test]
fn xml_is_pretty_as_markup_and_raw_as_text() {
    let raw = "<r><a>1</a></r>";
    assert_eq!(
        prepare(BodyKind::Xml, raw, 15, true),
        Prepared::Editor {
            text: "<r>\n  <a>1</a>\n</r>".into(),
            language: "html"
        }
    );
    assert_eq!(
        prepare(BodyKind::Xml, raw, 15, false),
        Prepared::Editor {
            text: raw.into(),
            language: "text"
        }
    );
    assert_eq!(
        prepare(BodyKind::Xml, "<a><b></a>", 10, true),
        Prepared::Editor {
            text: "<a><b></a>".into(),
            language: "html"
        }
    );
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbaImage::new(w, h)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

#[test]
fn image_content_types_are_images_whatever_the_bytes() {
    for (ty, kind) in [
        ("image/png", ImageKind::Png),
        ("IMAGE/PNG; q=1", ImageKind::Png),
        ("image/jpeg", ImageKind::Jpeg),
        ("image/jpg", ImageKind::Jpeg),
        ("image/gif", ImageKind::Gif),
        ("image/webp", ImageKind::Webp),
        ("image/svg+xml", ImageKind::Svg),
        ("image/bmp", ImageKind::Bmp),
    ] {
        assert_eq!(body_kind(&ct(ty), b"<svg/>"), BodyKind::Image(kind), "{ty}");
        assert_eq!(
            body_kind(&ct(ty), &png(1, 1)),
            BodyKind::Image(kind),
            "{ty}"
        );
    }
    assert_eq!(body_kind(&ct("image/png"), b""), BodyKind::Empty);
    assert_eq!(
        body_kind(&ct("image/tiff"), &[0xff, 0xfe, 0]),
        BodyKind::Binary
    );
    assert_eq!(
        body_kind(&ct("application/octet-stream"), &png(1, 1)),
        BodyKind::Binary
    );
}

#[test]
fn image_info_reads_raster_dimensions_from_the_header() {
    let bytes = png(3, 2);
    assert_eq!(
        image_info(ImageKind::Png, &bytes),
        ImageInfo {
            kind: ImageKind::Png,
            size: bytes.len() as u64,
            dimensions: Some((3, 2))
        }
    );
    assert_eq!(image_info(ImageKind::Png, b"not a png").dimensions, None);
    assert_eq!(image_info(ImageKind::Jpeg, &bytes).dimensions, None);
}

#[test]
fn image_info_reads_svg_dimensions_from_its_size_or_view_box() {
    let sized = br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="120px" height="80"></svg>"#;
    assert_eq!(
        image_info(ImageKind::Svg, sized).dimensions,
        Some((120, 80))
    );
    let boxed = br#"<svg viewBox="0 0 24.4 16" xmlns="http://www.w3.org/2000/svg"/>"#;
    assert_eq!(image_info(ImageKind::Svg, boxed).dimensions, Some((24, 16)));
    let relative = br#"<svg width="100%" height="100%"/>"#;
    assert_eq!(image_info(ImageKind::Svg, relative).dimensions, None);
    assert_eq!(image_info(ImageKind::Svg, b"<html/>").dimensions, None);
}

#[test]
fn image_info_falls_back_to_the_view_box_for_unusable_svg_sizes() {
    let dims = |svg: &str| image_info(ImageKind::Svg, svg.as_bytes()).dimensions;
    for width in ["0", "-5", "NaN", "inf"] {
        let svg = format!(r#"<svg width="{width}" height="10" viewBox="0 0 30 20"/>"#);
        assert_eq!(dims(&svg), Some((30, 20)), "width={width}");
    }
    let no_height = r#"<svg width="50" viewBox="0 0 30 20"/>"#;
    assert_eq!(dims(no_height), Some((30, 20)));
    assert_eq!(dims(r#"<svg width="0" height="0"/>"#), None);
    assert_eq!(
        dims(r#"<svg width="0" height="9" viewBox="0 0 0 20"/>"#),
        None
    );
}

#[test]
fn image_caption_names_format_dimensions_and_size() {
    let info = ImageInfo {
        kind: ImageKind::Png,
        size: 2048,
        dimensions: Some((640, 480)),
    };
    assert_eq!(image_caption(&info), "PNG image, 640 × 480, 2.0 KB");
    let info = ImageInfo {
        kind: ImageKind::Svg,
        size: 512,
        dimensions: None,
    };
    assert_eq!(image_caption(&info), "SVG image, 512 B");
}

#[test]
fn prepare_body_previews_whole_images_only() {
    let bytes = png(3, 2);
    let kind = BodyKind::Image(ImageKind::Png);
    assert_eq!(
        prepare_body(kind, &bytes, "", true, false),
        Prepared::Image(image_info(ImageKind::Png, &bytes))
    );
    assert_eq!(
        prepare_body(kind, &bytes, "", false, false),
        Prepared::Image(image_info(ImageKind::Png, &bytes))
    );
    assert_eq!(
        prepare_body(kind, &bytes, "", true, true),
        Prepared::Binary {
            size: bytes.len() as u64
        }
    );
    assert_eq!(
        prepare_body(BodyKind::Text, b"hi", "hi", true, false),
        prepare(BodyKind::Text, "hi", 2, true)
    );
}

#[test]
fn beautify_json_keeps_key_order_and_number_text() {
    let got = beautify_json("{\"b\":1.50,\"a\":1e5,\"z\":[0.10,-0]}").unwrap();
    assert_eq!(
        got,
        "{\n  \"b\": 1.50,\n  \"a\": 1e5,\n  \"z\": [\n    0.10,\n    -0\n  ]\n}"
    );
}

#[test]
fn beautify_json_keeps_escapes_and_string_contents() {
    let got = beautify_json("{\"k\":\"\\u00e9 é \\\"q\\\" a,{b}: [\"}").unwrap();
    assert_eq!(got, "{\n  \"k\": \"\\u00e9 é \\\"q\\\" a,{b}: [\"\n}");
}

#[test]
fn beautify_json_keeps_nested_empty_containers_compact() {
    let got = beautify_json("{\"a\":{ },\"b\":[\n],\"c\":[{},[]]}").unwrap();
    assert_eq!(
        got,
        "{\n  \"a\": {},\n  \"b\": [],\n  \"c\": [\n    {},\n    []\n  ]\n}"
    );
}

#[test]
fn beautify_json_is_idempotent() {
    let once = beautify_json("{\"a\":[1,{\"b\":null}],\"c\":\"x\"}").unwrap();
    assert_eq!(beautify_json(&once).unwrap(), once);
    let holes = beautify_json("{\"n\":{{count}},\"a\":[{{x}},1]}").unwrap();
    assert_eq!(beautify_json(&holes).unwrap(), holes);
}

#[test]
fn beautify_json_names_where_invalid_input_breaks() {
    let err = beautify_json("{\n  \"a\": ,\n}").unwrap_err();
    assert!(err.contains("line 2"), "{err}");
    assert!(beautify_json("not json").is_err());
    assert!(beautify_json("{\"a\":1").is_err());
    assert!(beautify_json("").is_err());
}

#[test]
fn beautify_json_formats_unquoted_placeholders_as_values() {
    let got = beautify_json("{\"n\": {{count}}, \"a\": [{{x}}, 1], \"o\":{{ obj }}}").unwrap();
    assert_eq!(
        got,
        "{\n  \"n\": {{count}},\n  \"a\": [\n    {{x}},\n    1\n  ],\n  \"o\": {{ obj }}\n}"
    );
}

#[test]
fn beautify_json_keeps_placeholders_in_strings_and_rejects_misplaced_ones() {
    let got = beautify_json("{\"u\":\"{{base}}/x\",\"k\":\"{{a}}\"}").unwrap();
    assert_eq!(got, "{\n  \"u\": \"{{base}}/x\",\n  \"k\": \"{{a}}\"\n}");
    // A placeholder with no comma between values is still a syntax error.
    assert!(beautify_json("[{{a}} {{b}}]").is_err());
    assert!(beautify_json("[1{{a}}]").is_err());
    assert!(beautify_json("{{a}}: 1").is_err());
}

#[test]
fn beautify_json_error_columns_count_placeholders_as_written() {
    let err = beautify_json("{\"n\": {{count}}, \"a\": }").unwrap_err();
    assert!(err.contains("line 1 column 23"), "{err}");
}

#[test]
fn beautify_xml_reindents_and_keeps_placeholders() {
    let got = beautify_xml("<r a=\"{{id}}\"><t>{{name}}</t><b/></r>").unwrap();
    assert_eq!(got, "<r a=\"{{id}}\">\n  <t>{{name}}</t>\n  <b/>\n</r>");
}

#[test]
fn beautify_xml_reports_malformed_input_with_a_position() {
    let err = beautify_xml("<a>\n<b></a>").unwrap_err();
    assert!(err.contains("line 2"), "{err}");
    assert!(beautify_xml("<a><b>").is_err());
}

fn joined(c: &Chunked) -> String {
    let mut out = String::new();
    for (row, no) in c.rows.iter().zip(&c.line_nos) {
        if *no != 0 && !out.is_empty() {
            out.push('\n');
        }
        out.push_str(row);
    }
    out
}

#[test]
fn chunking_numbers_only_the_first_chunk_of_a_line() {
    let c = chunk_lines("abcdefghij\nxy\n\nklm", 4);
    let rows: Vec<&str> = c.rows.iter().map(|r| r.as_ref()).collect();
    assert_eq!(rows, ["abcd", "efgh", "ij", "xy", "", "klm"]);
    assert_eq!(c.line_nos, [1, 0, 0, 2, 3, 4]);
    assert_eq!(c.line_count(), 4);
    assert_eq!(joined(&c), "abcdefghij\nxy\n\nklm");
}

#[test]
fn rechunking_changes_the_width_and_keeps_the_text() {
    let text = format!("{}\nshort\n{}", "é".repeat(700), "z".repeat(2500));
    let wide = chunk_lines(&text, ROW_WIDTH);
    for width in [8, 13, 80, 400, ROW_WIDTH] {
        let narrow = wide.rechunk(width);
        assert_eq!(joined(&narrow), text.replace('\r', ""), "{width}");
        assert!(
            narrow.rows.iter().all(|r| r.len() <= width.max(2)),
            "{width}"
        );
        assert_eq!(narrow.line_nos.iter().filter(|n| **n != 0).count(), 3);
        assert_eq!(narrow, chunk_lines(&text, width), "{width}");
    }
}

#[test]
fn the_widest_row_is_found() {
    let c = chunk_lines("a\nbbbb\ncc", 3);
    assert_eq!(c.widest(), 1);
}

#[test]
fn wrap_columns_quantise_to_eights_and_cap_at_the_row_width() {
    use gpui_kit::px;
    assert_eq!(wrap_columns(px(805.), px(8.)), 96);
    assert_eq!(wrap_columns(px(808.), px(8.)), 96);
    assert_eq!(wrap_columns(px(10.), px(8.)), 8);
    assert_eq!(wrap_columns(px(100_000.), px(8.)), ROW_WIDTH);
    assert_eq!(wrap_columns(px(500.), px(0.)), ROW_WIDTH);
}

#[test]
fn minify_json_round_trips_a_pretty_fixture_byte_for_byte() {
    let pretty = "{\n  \"b\": 1.50,\n  \"a\": [\n    {{x}},\n    12345678901234567890,\n    {},\n    []\n  ],\n  \"s\": \"a  b\\n {{y}} \"\n}";
    let min = minify_json(pretty).unwrap();
    assert_eq!(
        min,
        "{\"b\":1.50,\"a\":[{{x}},12345678901234567890,{},[]],\"s\":\"a  b\\n {{y}} \"}"
    );
    assert_eq!(beautify_json(&min).unwrap(), pretty);
}

#[test]
fn minify_json_keeps_key_order_numbers_and_escapes() {
    let got =
        minify_json("{ \"z\" : 1e5 ,\n \"a\" : [ 0.10 , -0 ] , \"k\" : \"\\u00e9 é \\\"q\\\"\" }")
            .unwrap();
    assert_eq!(
        got,
        "{\"z\":1e5,\"a\":[0.10,-0],\"k\":\"\\u00e9 é \\\"q\\\"\"}"
    );
}

#[test]
fn minify_json_error_matches_beautify() {
    for bad in ["{\n  \"a\": ,\n}", "not json", "", "[{{a}} {{b}}]"] {
        assert_eq!(minify_json(bad), beautify_json(bad));
        assert!(minify_json(bad).is_err());
    }
}

#[test]
fn minify_xml_drops_layout_whitespace_only() {
    let got =
        minify_xml("<r a=\"{{id}}\">\n  <t> hi there </t>\n  <!-- c -->\n  <b/>\n</r>").unwrap();
    assert_eq!(got, "<r a=\"{{id}}\"><t> hi there </t><!-- c --><b/></r>");
}

#[test]
fn minify_xml_keeps_cdata_and_reports_malformed_input_like_beautify() {
    let got = minify_xml("<a>\n  <![CDATA[ x  y ]]>\n</a>").unwrap();
    assert_eq!(got, "<a><![CDATA[ x  y ]]></a>");
    assert_eq!(minify_xml("<a>\n<b></a>"), beautify_xml("<a>\n<b></a>"));
    assert!(minify_xml("<a><b>").is_err());
}

#[test]
fn minify_xml_keeps_whitespace_between_inline_siblings_in_mixed_content() {
    let src = "<p>Hi <b>x</b> <i>y</i></p>";
    assert_eq!(minify_xml(src).unwrap(), src);
    assert_eq!(beautify_xml(src).unwrap(), src);
}

#[test]
fn mixed_content_keeps_the_space_between_inline_siblings() {
    for src in ["<p><b>x</b> <i>y</i></p>", "<p>Hi <b>x</b> <i>y</i></p>"] {
        assert_eq!(beautify_xml(src).unwrap(), src);
        assert_eq!(minify_xml(src).unwrap(), src);
        assert_eq!(reindent_xml(src).unwrap(), src);
    }
}

#[test]
fn mixed_content_keeps_newline_whitespace_and_adds_no_indentation() {
    let src = "<p>Hi <b>x</b>\n  <i>y</i></p>";
    assert_eq!(beautify_xml(src).unwrap(), src);
    assert_eq!(minify_xml(src).unwrap(), src);
    let once = beautify_xml(src).unwrap();
    assert_eq!(beautify_xml(&once).unwrap(), once);
}

#[test]
fn mixed_content_with_an_entity_reference_counts_as_mixed() {
    let src = "<p>&amp;<b>x</b> <i>y</i></p>";
    assert_eq!(beautify_xml(src).unwrap(), src);
}

#[test]
fn element_only_content_is_still_compacted_and_indented() {
    let src = "<r>\n  <a>1</a>\n  <b>2</b>\n</r>";
    assert_eq!(minify_xml(src).unwrap(), "<r><a>1</a><b>2</b></r>");
    assert_eq!(
        beautify_xml(src).unwrap(),
        "<r>\n  <a>1</a>\n  <b>2</b>\n</r>"
    );
}

#[test]
fn a_mixed_element_inside_element_only_content_is_indented_as_a_unit() {
    let src = "<r><p>Hi <b>x</b> <i>y</i></p><q>z</q></r>";
    assert_eq!(
        beautify_xml(src).unwrap(),
        "<r>\n  <p>Hi <b>x</b> <i>y</i></p>\n  <q>z</q>\n</r>"
    );
    assert_eq!(minify_xml(src).unwrap(), src);
}

#[test]
fn element_only_content_nested_in_a_mixed_element_adds_no_indentation() {
    let src = "<p>Hi <span>\n  <a>1</a>\n  <b>2</b>\n</span> end</p>";
    let want = "<p>Hi <span><a>1</a><b>2</b></span> end</p>";
    assert_eq!(beautify_xml(src).unwrap(), want);
    assert_eq!(minify_xml(src).unwrap(), want);
}

#[test]
fn xml_space_preserve_keeps_whitespace_and_inherits_until_default() {
    let src = "<r><a xml:space=\"preserve\"><b>1</b> <c>2</c>\n <d><e>3</e> <f/></d></a></r>";
    assert_eq!(
        beautify_xml(src).unwrap(),
        "<r>\n  <a xml:space=\"preserve\"><b>1</b> <c>2</c>\n <d><e>3</e> <f/></d></a>\n</r>"
    );
    assert_eq!(minify_xml(src).unwrap(), src);
    let over =
        "<r><a xml:space=\"preserve\"><b>1</b> <d xml:space=\"default\"><e>3</e>\n<f/></d></a></r>";
    assert_eq!(
        minify_xml(over).unwrap(),
        "<r><a xml:space=\"preserve\"><b>1</b> <d xml:space=\"default\"><e>3</e><f/></d></a></r>"
    );
}

#[test]
fn json_output_is_unaffected_by_xml_mixed_content_handling() {
    assert_eq!(minify_json("{ \"a\": [1, 2] }").unwrap(), "{\"a\":[1,2]}");
    assert_eq!(beautify_json("{\"a\":1}").unwrap(), "{\n  \"a\": 1\n}");
}

#[test]
fn minify_xml_keeps_declaration_entities_comments_and_sibling_cdata() {
    let src = "<?xml version=\"1.0\"?>\n<r>\n  <!-- c -->\n  <t>a &amp; b</t>\n  <![CDATA[ x ]]>\n  <![CDATA[ y ]]>\n</r>";
    let got = minify_xml(src).unwrap();
    assert!(got.starts_with("<?xml version=\"1.0\"?>"));
    assert!(got.contains("<!-- c -->"));
    assert!(got.contains("<t>a &amp; b</t>"));
    assert!(got.contains("<![CDATA[ x ]]><![CDATA[ y ]]>"));
    assert_eq!(beautify_xml(&got), beautify_xml(src));
}

#[test]
fn pretty_html_is_reindented_and_raw_is_untouched() {
    let html = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>T</title></head>\
                <body><div class=\"a\"><p>Hello   <b>there</b></p><br><img src=\"x\"/><span></span></div>\
                <!-- note --></body></html>";
    let pretty = "<!DOCTYPE html>\n<html>\n  <head>\n    <meta charset=\"utf-8\">\n    \
                  <title>T</title>\n  </head>\n  <body>\n    <div class=\"a\">\n      <p>\n        \
                  Hello\n        <b>there</b>\n      </p>\n      <br>\n      <img src=\"x\"/>\n      \
                  <span></span>\n    </div>\n    <!-- note -->\n  </body>\n</html>";
    assert_eq!(
        display_text(BodyKind::Html, html, true),
        Some((pretty.to_owned(), "html"))
    );
    assert_eq!(
        display_text(BodyKind::Html, html, false),
        Some((html.to_owned(), "text"))
    );
}

#[test]
fn pretty_html_keeps_script_and_pre_content() {
    let html = "<body><pre>  a\n   b</pre><script>\n    if (a < b) {\n      go();\n    }\n</script></body>";
    let pretty = "<body>\n  <pre>  a\n   b</pre>\n  <script>\n    if (a < b) {\n      go();\n    }\n  </script>\n</body>";
    assert_eq!(reindent_html(html), pretty);
}

#[test]
fn pretty_html_tolerates_stray_and_unclosed_tags() {
    assert_eq!(reindent_html("<p>"), "<p>");
    assert_eq!(
        reindent_html("<div><p>a</div>x"),
        "<div>\n  <p>\n    a\n</div>\nx"
    );
    assert_eq!(reindent_html("</i>a < b"), "</i>\na < b");
    assert_eq!(reindent_html("  plain  "), "plain");
}

// Unterminated tags and comments end the scan, so a hostile body stays linear.
#[test]
fn pretty_html_stays_linear_on_unterminated_tags_and_comments() {
    let tags = "<a".repeat(200_000);
    assert_eq!(reindent_html(&tags), tags);
    let comments = "<!--".repeat(200_000);
    assert_eq!(reindent_html(&comments), comments);
    let quotes = "<a x=\"".repeat(100_000);
    assert_eq!(reindent_html(&quotes).len(), quotes.len());
    let stray = format!("{}{}", "<div>".repeat(50_000), "</p>".repeat(50_000));
    assert_eq!(reindent_html(&stray).lines().count(), 100_000);
}

#[test]
fn pretty_html_indents_at_most_32_levels() {
    let deep = "<div>".repeat(50_000);
    let out = reindent_html(&deep);
    let widest = out.lines().map(|l| l.len() - l.trim_start().len()).max();
    assert_eq!(widest, Some(64));
    assert!(out.len() <= 50_000 * 70, "{}", out.len());
}

#[test]
fn pretty_html_handles_quotes_comments_case_and_non_ascii() {
    assert_eq!(
        reindent_html("<div><a title=\"x>y\">t</a></div>"),
        "<div>\n  <a title=\"x>y\">t</a>\n</div>"
    );
    assert_eq!(
        reindent_html("<div><!-- <p>no</p> --></div>"),
        "<div>\n  <!-- <p>no</p> -->\n</div>"
    );
    assert_eq!(
        reindent_html("<DIV><P>a</P><SCRIPT>x < y</SCRIPT></DIV>"),
        "<DIV>\n  <P>a</P>\n  <SCRIPT>\n    x < y\n  </SCRIPT>\n</DIV>"
    );
    assert_eq!(
        reindent_html("<p>héllo  wörld 日本</p><p>ü</p>"),
        "<p>héllo wörld 日本</p>\n<p>ü</p>"
    );
}

#[test]
fn a_raw_element_ends_only_at_its_own_name() {
    assert_eq!(
        reindent_html("<script>a</scripts>b</script>"),
        "<script>\n  a</scripts>b\n</script>"
    );
}
