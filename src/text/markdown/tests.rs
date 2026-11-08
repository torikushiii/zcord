use super::*;
use std::time::{Duration, Instant};

fn text(value: &str) -> Span {
    Span {
        kind: SpanKind::Text(value.to_owned()),
        style: Style::default(),
        link: None,
    }
}

fn paragraph(value: &str) -> Block {
    Block::Paragraph(vec![text(value)])
}

fn spans(value: &str) -> Vec<Span> {
    let parsed = parse(value);
    let [Block::Paragraph(spans)] = parsed.as_slice() else {
        panic!("expected paragraph, got {parsed:?}");
    };
    spans.clone()
}

fn code(language: Option<&str>, value: &str) -> Block {
    Block::CodeBlock {
        language: language.map(str::to_owned),
        code: value.to_owned(),
    }
}

#[test]
fn markdown_empty_message() {
    assert!(parse("").is_empty());
    assert!(parse("\n\n").is_empty());
    assert!(!is_jumbo(&[]));
}

#[test]
fn markdown_preserves_paragraph_newlines() {
    assert_eq!(
        parse("\nhello\n\n\nworld\n"),
        vec![paragraph("hello\n\n\nworld")]
    );
    assert_eq!(parse("  hello  "), vec![paragraph("  hello  ")]);
}

#[test]
fn markdown_consumes_exactly_one_newline_after_blocks() {
    assert_eq!(
        parse("# title\n\ntext"),
        vec![
            Block::Heading {
                level: 1,
                spans: vec![text("title")]
            },
            paragraph("text")
        ]
    );
    assert_eq!(
        parse("```code```\n\ntext"),
        vec![code(None, "code"), paragraph("\ntext")]
    );
    assert_eq!(
        parse("text\n\n# title"),
        vec![
            paragraph("text\n\n"),
            Block::Heading {
                level: 1,
                spans: vec![text("title")]
            }
        ]
    );
}

#[test]
fn markdown_code_block_language() {
    assert_eq!(
        parse("```rust\nlet x = 1;\n```"),
        vec![code(Some("rust"), "let x = 1;")]
    );
    for lang in ["c++", "c#", "objective-c", "lang_v1.2", "123"] {
        assert_eq!(
            parse(&format!("```{lang}\nx```")),
            vec![code(Some(lang), "x")]
        );
    }
}

#[test]
fn markdown_code_block_no_language() {
    assert_eq!(parse("```\nhello\n```"), vec![code(None, "\nhello")]);
    assert_eq!(
        parse("```not a language\nhello\n```"),
        vec![code(None, "not a language\nhello")]
    );
    assert_eq!(
        parse("``` rust\nhello```"),
        vec![code(None, " rust\nhello")]
    );
}

#[test]
fn markdown_code_block_one_line_and_mid_line() {
    assert_eq!(parse("```hello```"), vec![code(None, "hello")]);
    assert_eq!(
        parse("before ```hello``` after"),
        vec![
            paragraph("before "),
            code(None, "hello"),
            paragraph(" after")
        ]
    );
    assert_eq!(parse("``````"), vec![code(None, "")]);
}

#[test]
fn markdown_code_block_literal_and_trailing_newline() {
    assert_eq!(
        parse("```rs\n**x** <@1> ||x||\n\n```"),
        vec![code(Some("rs"), "**x** <@1> ||x||\n")]
    );
}

#[test]
fn markdown_unclosed_code_fence() {
    assert_eq!(
        parse("```hello\n# heading"),
        vec![
            paragraph("```hello\n"),
            Block::Heading {
                level: 1,
                spans: vec![text("heading")]
            }
        ]
    );
}

#[test]
fn markdown_heading_levels() {
    for level in 1..=3 {
        assert_eq!(
            parse(&format!("{} title", "#".repeat(level))),
            vec![Block::Heading {
                level: level as u8,
                spans: vec![text("title")]
            }]
        );
    }
}

#[test]
fn markdown_heading_requires_valid_prefix_and_content() {
    for value in ["#### x", "#x", "# ", "##   ", "before # title", " # title"] {
        assert_eq!(parse(value), vec![paragraph(value)]);
    }
}

#[test]
fn markdown_heading_and_subtext_inline_styles() {
    let bold = Span {
        style: Style {
            bold: true,
            ..Style::default()
        },
        ..text("title")
    };
    assert_eq!(
        parse("# **title**\n-# **title**"),
        vec![
            Block::Heading {
                level: 1,
                spans: vec![bold.clone()]
            },
            Block::Subtext(vec![bold])
        ]
    );
    assert_eq!(parse("-# "), vec![paragraph("-# ")]);
}

#[test]
fn markdown_consecutive_quote_lines() {
    assert_eq!(
        parse("> a\n> b\ntext"),
        vec![Block::Quote(vec![paragraph("a\nb")]), paragraph("text")]
    );
    assert_eq!(
        parse("before\n> a"),
        vec![paragraph("before\n"), Block::Quote(vec![paragraph("a")])]
    );
}

#[test]
fn markdown_empty_quote_lines_and_space_requirement() {
    assert_eq!(
        parse(">\n> a\n>\n> b"),
        vec![Block::Quote(vec![paragraph("\na\n\nb")])]
    );
    assert_eq!(parse(">"), vec![Block::Quote(vec![])]);
    assert_eq!(parse(">text"), vec![paragraph(">text")]);
}

#[test]
fn markdown_multiline_quote() {
    assert_eq!(
        parse(">>> first\nsecond\n# heading"),
        vec![Block::Quote(vec![
            paragraph("first\nsecond\n"),
            Block::Heading {
                level: 1,
                spans: vec![text("heading")]
            }
        ])]
    );
}

#[test]
fn markdown_quotes_do_not_nest() {
    assert_eq!(
        parse("> > inner\n> >>> still text"),
        vec![Block::Quote(vec![paragraph("> inner\n>>> still text")])]
    );
}

#[test]
fn markdown_quote_contains_blocks() {
    assert_eq!(
        parse("> # title\n> - a\n> - b\n> ```rs\n> x\n> ```"),
        vec![Block::Quote(vec![
            Block::Heading {
                level: 1,
                spans: vec![text("title")]
            },
            Block::List {
                start: None,
                items: vec![vec![paragraph("a")], vec![paragraph("b")]]
            },
            code(Some("rs"), "x"),
        ])]
    );
}

#[test]
fn markdown_bullet_lists() {
    assert_eq!(
        parse("- a\n* b\nend"),
        vec![
            Block::List {
                start: None,
                items: vec![vec![paragraph("a")], vec![paragraph("b")]]
            },
            paragraph("end")
        ]
    );
    assert_eq!(
        spans("*bold*")[0].style,
        Style {
            italic: true,
            ..Style::default()
        }
    );
}

#[test]
fn markdown_ordered_list_start_and_digit_limit() {
    assert_eq!(
        parse("42. a\n99. b"),
        vec![Block::List {
            start: Some(42),
            items: vec![vec![paragraph("a")], vec![paragraph("b")]]
        }]
    );
    assert_eq!(parse("1234567890. a"), vec![paragraph("1234567890. a")]);
}

#[test]
fn markdown_nested_lists_and_dedent() {
    assert_eq!(
        parse("- parent\n  3. child\n    - grandchild\n  4. sibling\n- next"),
        vec![Block::List {
            start: None,
            items: vec![
                vec![
                    paragraph("parent"),
                    Block::List {
                        start: Some(3),
                        items: vec![
                            vec![
                                paragraph("child"),
                                Block::List {
                                    start: None,
                                    items: vec![vec![paragraph("grandchild")]]
                                }
                            ],
                            vec![paragraph("sibling")],
                        ]
                    }
                ],
                vec![paragraph("next")]
            ],
        }]
    );
}

#[test]
fn markdown_one_space_does_not_nest_lists() {
    assert_eq!(
        parse("- a\n - b"),
        vec![Block::List {
            start: None,
            items: vec![vec![paragraph("a")], vec![paragraph("b")]]
        }]
    );
}

#[test]
fn markdown_list_type_changes() {
    assert_eq!(
        parse("- a\n2. b\n- c"),
        vec![
            Block::List {
                start: None,
                items: vec![vec![paragraph("a")]]
            },
            Block::List {
                start: Some(2),
                items: vec![vec![paragraph("b")]]
            },
            Block::List {
                start: None,
                items: vec![vec![paragraph("c")]]
            },
        ]
    );
}

#[test]
fn markdown_escapes_ascii_punctuation() {
    assert_eq!(
        spans(r"\*x\* \\ \a \! \_ \|"),
        vec![text(r"*x* \ \a ! _ |")]
    );
    for c in (0u8..=127).filter(u8::is_ascii_punctuation) {
        let value = format!("\\{}", c as char);
        assert_eq!(spans(&value), vec![text(&(c as char).to_string())]);
    }
}

#[test]
fn markdown_inline_code_is_literal() {
    assert_eq!(
        spans("`**x** <@1> ||x||`"),
        vec![Span {
            kind: SpanKind::Code("**x** <@1> ||x||".to_owned()),
            style: Style::default(),
            link: None
        }]
    );
    assert_eq!(spans("` x `")[0].kind, SpanKind::Code(" x ".to_owned()));
}

#[test]
fn markdown_double_backtick_code() {
    assert_eq!(
        spans("`` x ` y ``")[0].kind,
        SpanKind::Code("x ` y".to_owned())
    );
    assert_eq!(spans("`` x``")[0].kind, SpanKind::Code(" x".to_owned()));
    assert_eq!(spans("``"), vec![text("``")]);
}

#[test]
fn markdown_code_does_not_process_backslash_escapes() {
    assert_eq!(
        spans(r"`a\` tail")[0].kind,
        SpanKind::Code("a\\".to_owned())
    );
    assert_eq!(spans(r"\`literal\`"), vec![text("`literal`")]);
}

#[test]
fn markdown_escaped_fence_is_literal() {
    assert_eq!(parse(r"\```hello```"), vec![paragraph("```hello```")]);
    assert_eq!(
        parse(r"\\```hello```"),
        vec![paragraph("\\"), code(None, "hello")]
    );
}

#[test]
fn markdown_spoilers_and_strike_allow_edge_spaces() {
    assert_eq!(
        spans("|| secret ||"),
        vec![Span {
            style: Style {
                spoiler: Some(0),
                ..Style::default()
            },
            ..text(" secret ")
        }]
    );
    assert_eq!(
        spans("~~ strike ~~"),
        vec![Span {
            style: Style {
                strikethrough: true,
                ..Style::default()
            },
            ..text(" strike ")
        }]
    );
    assert_eq!(
        spans("__ under __"),
        vec![Span {
            style: Style {
                underline: true,
                ..Style::default()
            },
            ..text(" under ")
        }]
    );
}

#[test]
fn markdown_individual_styles() {
    for (value, expected) in [
        (
            "**x**",
            Style {
                bold: true,
                ..Style::default()
            },
        ),
        (
            "*x*",
            Style {
                italic: true,
                ..Style::default()
            },
        ),
        (
            "_x_",
            Style {
                italic: true,
                ..Style::default()
            },
        ),
        (
            "__x__",
            Style {
                underline: true,
                ..Style::default()
            },
        ),
        (
            "~~x~~",
            Style {
                strikethrough: true,
                ..Style::default()
            },
        ),
    ] {
        assert_eq!(
            spans(value),
            vec![Span {
                style: expected,
                ..text("x")
            }]
        );
    }
}

#[test]
fn markdown_triple_markers() {
    assert_eq!(
        spans("***x***")[0].style,
        Style {
            bold: true,
            italic: true,
            ..Style::default()
        }
    );
    assert_eq!(
        spans("___x___")[0].style,
        Style {
            underline: true,
            italic: true,
            ..Style::default()
        }
    );
}

#[test]
fn markdown_nested_styles() {
    let actual = spans("**bold *both* bold**");
    assert_eq!(actual.len(), 3);
    assert!(actual.iter().all(|span| span.style.bold));
    assert_eq!(
        actual[1],
        Span {
            style: Style {
                bold: true,
                italic: true,
                ..Style::default()
            },
            ..text("both")
        }
    );
    assert!(!actual[0].style.italic && !actual[2].style.italic);
    let all = spans("__**~~*x*~~**__");
    assert_eq!(
        all[0].style,
        Style {
            bold: true,
            italic: true,
            underline: true,
            strikethrough: true,
            spoiler: None
        }
    );
}

#[test]
fn markdown_shared_closing_marker_run() {
    assert_eq!(
        spans("**bold *both***")[1].style,
        Style {
            bold: true,
            italic: true,
            ..Style::default()
        }
    );
    assert_eq!(
        spans("*italic **both***")[1].style,
        Style {
            bold: true,
            italic: true,
            ..Style::default()
        }
    );
}

#[test]
fn markdown_snake_case_and_word_boundaries() {
    for value in ["snake_case_name", "word_x_", "_x_word", "word_x_word"] {
        assert_eq!(spans(value), vec![text(value)]);
    }
    assert!(spans("(_x_)")[1].style.italic);
}

#[test]
fn markdown_markers_require_nonspace_content() {
    for value in ["* x*", "*x *", "* *", "****", "||||", "~~~~"] {
        assert_eq!(
            Parser::default().inline(value, Style::default(), None, true),
            vec![text(value)]
        );
    }
}

#[test]
fn markdown_strong_allows_inner_whitespace() {
    for (value, expected) in [
        ("** x**", " x"),
        ("**x **", "x "),
        ("**5. No Self-Promotion **Please", "5. No Self-Promotion "),
    ] {
        let actual = spans(value);
        assert_eq!(actual[0].kind, SpanKind::Text(expected.into()));
        assert!(actual[0].style.bold);
    }
    let actual = spans("into the** language-specific** channels");
    assert_eq!(actual[1].kind, SpanKind::Text(" language-specific".into()));
    assert!(actual[1].style.bold);
}

#[test]
fn markdown_channel_links_preserve_their_original_url() {
    for host in [
        "discord.com",
        "ptb.discord.com",
        "canary.discord.com",
        "discordapp.com",
        "ptb.discordapp.com",
        "canary.discordapp.com",
    ] {
        let url = format!("https://{host}/channels/123/456");
        for content in [url.clone(), format!("<{url}>")] {
            let actual = spans(&content);
            assert_eq!(actual[0].kind, SpanKind::ChannelMention(456));
            assert_eq!(actual[0].link.as_deref(), Some(url.as_str()));
        }
        let actual = spans(&format!("[{url}](https://example.com)"));
        assert_eq!(actual[0].kind, SpanKind::Text(url.clone()));
        assert_eq!(actual[0].link.as_deref(), Some("https://example.com"));
        let message = format!("{url}/789");
        assert_eq!(
            spans(&message)[0].kind,
            SpanKind::MessageLink {
                channel_id: 456,
                message_id: 789
            }
        );
        assert_eq!(spans(&message)[0].link.as_deref(), Some(message.as_str()));
        let deeper = format!("{url}/789/1");
        assert_eq!(spans(&deeper)[0].kind, SpanKind::Text(deeper));
    }
    for url in [
        "https://discord.com.evil.test/channels/123/456",
        "https://example.com/channels/123/456",
        "https://discord.com/channels/no/456",
    ] {
        assert_eq!(spans(url)[0].kind, SpanKind::Text(url.into()));
    }
}

#[test]
fn markdown_formatting_fixture_mentions_strong_and_nested_lists() {
    fn collect_spans<'a>(blocks: &'a [Block], output: &mut Vec<&'a Span>) {
        for block in blocks {
            match block {
                Block::Paragraph(spans) | Block::Heading { spans, .. } | Block::Subtext(spans) => {
                    output.extend(spans)
                }
                Block::Quote(blocks) => collect_spans(blocks, output),
                Block::List { items, .. } => {
                    for blocks in items {
                        collect_spans(blocks, output);
                    }
                }
                Block::CodeBlock { .. } => {}
            }
        }
    }
    let message: crate::discord::model::Message =
        serde_json::from_str(include_str!("../../store/fixtures/formatting.json")).unwrap();
    let blocks = parse(&message.content);
    let mut actual = Vec::new();
    collect_spans(&blocks, &mut actual);
    for expected in ["5. No Self-Promotion ", " language-specific"] {
        assert!(
            actual
                .iter()
                .any(|span| span.kind == SpanKind::Text(expected.into()) && span.style.bold)
        );
    }
    assert!(
        actual
            .iter()
            .any(|span| span.kind == SpanKind::UserMention(1191169014890496024))
    );
    assert!(
        actual
            .iter()
            .any(|span| span.kind == SpanKind::RoleMention(1191169065222144036))
    );
    assert!(actual.iter().any(|span| matches!(&span.kind, SpanKind::Emoji { id: 1191169094582272043, name, animated: false } if name == "sample_icon")));
    assert!(actual.iter().any(
        |span| span.kind == SpanKind::ChannelMention(1191169073610752038)
            && span.link.as_deref()
                == Some("https://discord.com/channels/1191169056833536034/1191169073610752038")
    ));
    let lists: Vec<_> = blocks
        .iter()
        .filter_map(|block| match block {
            Block::List { items, .. } => Some(items),
            _ => None,
        })
        .collect();
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0].len(), 2);
    assert!(plain_text(&lists[0][0]).starts_with("Sample Community Mod:"));
    assert!(
        matches!(&lists[0][0][1], Block::List { items, .. } if items.len() == 1 && plain_text(&items[0]).starts_with("Community Mod:"))
    );
    assert!(plain_text(&lists[0][1]).starts_with("For Sample Community"));
    assert!(
        matches!(&lists[0][1][1], Block::List { items, .. } if items.len() == 2 && plain_text(&items[0]).starts_with("Discuss with"))
    );
}

#[test]
fn markdown_unmatched_markers_are_literal() {
    for value in [
        "**bold",
        "italic*",
        "__under",
        "~~strike",
        "||spoiler",
        "[link",
        "`code",
    ] {
        assert_eq!(spans(value), vec![text(value)]);
    }
    let actual = spans("**outer *inner*");
    assert_eq!(actual[0], text("**outer "));
    assert!(actual[1].style.italic && !actual[1].style.bold);
}

#[test]
fn markdown_spoiler_indices_follow_source_order_across_blocks() {
    let parsed = parse("||a **b**||\n# ||c||\n> ||d||\n- ||e||\n- [||f||](https://example.com)");
    let mut indices = Vec::new();
    let mut pending: Vec<&Block> = parsed.iter().rev().collect();
    while let Some(block) = pending.pop() {
        match block {
            Block::Paragraph(spans) | Block::Heading { spans, .. } | Block::Subtext(spans) => {
                indices.extend(spans.iter().filter_map(|span| span.style.spoiler));
            }
            Block::Quote(blocks) => pending.extend(blocks.iter().rev()),
            Block::List { items, .. } => {
                pending.extend(items.iter().rev().flat_map(|item| item.iter().rev()))
            }
            Block::CodeBlock { .. } => {}
        }
    }
    assert_eq!(indices, vec![0, 0, 1, 2, 3, 4]);
}

#[test]
fn markdown_unmatched_spoiler_does_not_reserve_index() {
    let actual = spans("||unmatched [||inside||](https://example.com)");
    assert_eq!(actual[0], text("||unmatched "));
    assert_eq!(actual[1].style.spoiler, Some(0));
}

#[test]
fn markdown_autolinks_trim_punctuation() {
    let actual = spans("https://example.com.,:;!?\"'");
    assert_eq!(
        actual[0].kind,
        SpanKind::Text("https://example.com".to_owned())
    );
    assert_eq!(actual[0].link.as_deref(), Some("https://example.com"));
    assert_eq!(actual[1], text(".,:;!?\"'"));
}

#[test]
fn markdown_autolinks_balance_parentheses() {
    let actual = spans("(https://example.com/a(b))).");
    assert_eq!(actual[1].link.as_deref(), Some("https://example.com/a(b)"));
    assert_eq!(actual[2], text("))."));
}

#[test]
fn markdown_angle_links() {
    let actual = spans("<https://example.com> <http://example.com>");
    assert_eq!(
        actual[0].kind,
        SpanKind::Text("https://example.com".to_owned())
    );
    assert_eq!(actual[0].link.as_deref(), Some("https://example.com"));
    assert_eq!(actual[2].link.as_deref(), Some("http://example.com"));
}

#[test]
fn markdown_masked_links_preserve_styles_and_emoji() {
    let actual = spans("[**label** <:face:1>](https://example.com/a(b))");
    assert_eq!(actual.len(), 3);
    assert!(actual[0].style.bold);
    assert!(matches!(actual[2].kind, SpanKind::Emoji { id: 1, .. }));
    assert!(
        actual
            .iter()
            .all(|span| span.link.as_deref() == Some("https://example.com/a(b)"))
    );
}

#[test]
fn markdown_masked_angle_url_and_url_label() {
    let actual = spans("[https://label.example](<https://target.example>)");
    assert_eq!(
        actual,
        vec![Span {
            link: Some("https://target.example".to_owned()),
            ..text("https://label.example")
        }]
    );
}

#[test]
fn markdown_formatting_around_bare_urls_closes_before_them() {
    let bold = spans("**https://irlcat.com**");
    assert_eq!(bold.len(), 1);
    assert!(bold[0].style.bold);
    assert_eq!(
        bold[0].kind,
        SpanKind::Text("https://irlcat.com".to_owned())
    );
    assert_eq!(bold[0].link.as_deref(), Some("https://irlcat.com"));

    let path = spans("_hi https://a.example/foo_bar");
    assert_eq!(
        path.last().and_then(|span| span.link.as_deref()),
        Some("https://a.example/foo_bar")
    );
}

#[test]
fn bare_urls_keep_single_delimiters_inside_double_formatting() {
    for (marker, url) in [
        ("**", "https://a.example/a*b"),
        ("||", "https://a.example/a|b"),
        ("~~", "https://a.example/a~b"),
    ] {
        let parsed = spans(&format!("{marker}{url}{marker}"));
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].link.as_deref(), Some(url));
    }
    let url = format!("https://a.example/{}end", "_".repeat(4096));
    let parsed = spans(&format!("_hi {url}"));
    assert_eq!(
        parsed.last().and_then(|span| span.link.as_deref()),
        Some(url.as_str())
    );
}

#[test]
fn markdown_link_labels_do_not_nest_links() {
    let actual = spans("[[inner](https://inner.example) <@1>](https://outer.example)");
    assert_eq!(
        actual[0].kind,
        SpanKind::Text("[inner](https://inner.example) ".to_owned())
    );
    assert!(matches!(actual[1].kind, SpanKind::UserMention(1)));
    assert!(
        actual
            .iter()
            .all(|span| span.link.as_deref() == Some("https://outer.example"))
    );
}

#[test]
fn markdown_invalid_links_remain_text() {
    for value in [
        "[x](ftp://example.com)",
        "[x](https://)",
        "[](ftp://example.com)",
        "ftp://example.com",
        "https://",
    ] {
        assert_eq!(spans(value), vec![text(value)]);
    }
}

#[test]
fn markdown_mentions() {
    let actual = spans("<@1><@!2><@&3><#4>");
    assert_eq!(
        actual
            .iter()
            .map(|span| span.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            SpanKind::UserMention(1),
            SpanKind::UserMention(2),
            SpanKind::RoleMention(3),
            SpanKind::ChannelMention(4)
        ]
    );
}

#[test]
fn markdown_mentions_require_u64_ids() {
    for value in ["<@18446744073709551616>", "<@-1>", "<@+1>", "<#x>", "<@&>"] {
        assert_eq!(spans(value), vec![text(value)]);
    }
    assert_eq!(
        spans("<@18446744073709551615>")[0].kind,
        SpanKind::UserMention(u64::MAX)
    );
}

#[test]
fn markdown_everyone_and_here_word_boundaries() {
    assert_eq!(
        spans("@everyone @here")
            .iter()
            .map(|span| span.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            SpanKind::Everyone,
            SpanKind::Text(" ".to_owned()),
            SpanKind::Here
        ]
    );
    for value in ["@everyonex", "@here2", "email@here", "@here_name"] {
        assert_eq!(spans(value), vec![text(value)]);
    }
}

#[test]
fn markdown_custom_emoji() {
    assert_eq!(
        spans("<:face_1:42><a:wave:43>")
            .iter()
            .map(|span| span.kind.clone())
            .collect::<Vec<_>>(),
        vec![
            SpanKind::Emoji {
                id: 42,
                name: "face_1".to_owned(),
                animated: false
            },
            SpanKind::Emoji {
                id: 43,
                name: "wave".to_owned(),
                animated: true
            },
        ]
    );
    for value in [
        "<::1>",
        "<:bad-name:1>",
        "<:name:x>",
        "<a:name:18446744073709551616>",
    ] {
        assert_eq!(spans(value), vec![text(value)]);
    }
    let too_long = format!("<:{}:1>", "x".repeat(33));
    assert_eq!(spans(&too_long), vec![text(&too_long)]);
}

#[test]
fn markdown_timestamps() {
    assert_eq!(
        spans("<t:1700000000>")[0].kind,
        SpanKind::Timestamp {
            unix: 1700000000,
            style: 'f'
        }
    );
    for style in ['t', 'T', 'd', 'D', 'f', 'F', 'R'] {
        assert_eq!(
            spans(&format!("<t:-123:{style}>"))[0].kind,
            SpanKind::Timestamp { unix: -123, style }
        );
    }
    assert_eq!(
        spans("<t:-9223372036854775808:R>")[0].kind,
        SpanKind::Timestamp {
            unix: i64::MIN,
            style: 'R'
        }
    );
}

#[test]
fn markdown_invalid_timestamps() {
    for value in [
        "<t:1:X>",
        "<t:1:>",
        "<t:1:RR>",
        "<t:+1>",
        "<t:9223372036854775808>",
        "<t:->",
    ] {
        assert_eq!(spans(value), vec![text(value)]);
    }
}

#[test]
fn markdown_command_mentions() {
    for name in ["cmd", "cmd sub", "cmd sub group"] {
        assert_eq!(
            spans(&format!("</{name}:42>"))[0].kind,
            SpanKind::Command {
                name: name.to_owned(),
                id: 42
            }
        );
    }
    for value in ["</:1>", "</cmd  sub:1>", "</cmd a b c:1>", "</cmd:no>"] {
        assert_eq!(spans(value), vec![text(value)]);
    }
}

#[test]
fn markdown_special_spans_inherit_style_and_link() {
    let actual = spans(
        "[**<@1><#2><@&3>@everyone @here <:face:4><t:1:R></cmd:5>`code`**](https://example.com)",
    );
    assert!(
        actual
            .iter()
            .all(|span| span.style.bold && span.link.as_deref() == Some("https://example.com"))
    );
    assert!(
        actual
            .iter()
            .any(|span| matches!(span.kind, SpanKind::Code(_)))
    );
    assert!(
        actual
            .iter()
            .any(|span| matches!(span.kind, SpanKind::Command { .. }))
    );
}

#[test]
fn markdown_merges_adjacent_text() {
    assert_eq!(spans(r"a\!b\*c"), vec![text("a!b*c")]);
    assert_eq!(
        spans("**a****b**"),
        vec![Span {
            style: Style {
                bold: true,
                ..Style::default()
            },
            ..text("ab")
        }]
    );
    let linked = spans("[a](https://example.com)[b](https://example.com)");
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].kind, SpanKind::Text("ab".to_owned()));
}

#[test]
fn markdown_jumbo_custom_emoji() {
    assert!(is_jumbo(&parse("<:a:1> <a:b:2>")));
    assert!(is_jumbo(&parse(&"<:a:1>".repeat(30))));
    assert!(!is_jumbo(&parse(&"<:a:1>".repeat(31))));
}

#[test]
fn markdown_jumbo_unicode_sequences_and_flags() {
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let flag = "\u{1F1EE}\u{1F1E9}";
    assert!(is_jumbo(&parse(&format!("{family} {flag}"))));
    assert!(is_jumbo(&parse(&family.repeat(30))));
    assert!(is_jumbo(&parse(&flag.repeat(30))));
    assert!(!is_jumbo(&parse(&flag.repeat(31))));
    assert!(!is_jumbo(&parse("\u{1F1EE}")));
}

#[test]
fn markdown_jumbo_modifiers_keycaps_and_tags() {
    for value in [
        "\u{1F44D}\u{1F3FD}",
        "1\u{FE0F}\u{20E3}",
        "#\u{20E3}",
        "*\u{FE0F}\u{20E3}",
        "\u{00A9}\u{FE0E}",
        "\u{1F3F4}\u{E0067}\u{E0062}\u{E007F}",
    ] {
        assert!(is_jumbo(&parse(value)), "{value:?}");
    }
    for value in [
        "\u{200D}",
        "\u{FE0F}",
        "\u{1F3FD}",
        "\u{E0067}",
        "1",
        "#",
        "\u{1F600}\u{200D}",
    ] {
        assert!(!is_jumbo(&parse(value)), "{value:?}");
    }
}

#[test]
fn markdown_jumbo_mixed_and_nonemoji() {
    assert!(is_jumbo(&parse("<:face:1> \u{1F600}\n\u{2764}\u{FE0F}")));
    for value in [
        "text \u{1F600}",
        "<@1>",
        "`\u{1F600}`",
        "# \u{1F600}",
        "   ",
        "\u{1F600}\n# x",
    ] {
        assert!(!is_jumbo(&parse(value)), "{value:?}");
    }
    assert!(!is_jumbo(&parse(&"\u{1F600}".repeat(31))));
}

#[test]
fn markdown_plain_text_all_span_kinds() {
    let value =
        "**hi** `code` <@1> <@!2> <#3> <@&4> @everyone @here <:face:5> <t:6> <t:-7:R> </cmd sub:8>";
    assert_eq!(
        plain_text(&parse(value)),
        "hi code @1 @2 #3 @&4 @everyone @here :face: <t:6:f> <t:-7:R> /cmd sub"
    );
}

#[test]
fn markdown_plain_text_blocks_and_whitespace() {
    assert_eq!(
        plain_text(&parse(
            "# title\n-# sub\n> quote\n- first\n  - child\n- second\n```rs\nlet x\n= 1;\n```\nend\t  here"
        )),
        "title sub quote first child second let x = 1; end here"
    );
    assert_eq!(
        plain_text(&parse("[label](https://example.com)\n\n rest")),
        "label rest"
    );
}

#[test]
fn markdown_fuzz_fixed_seed() {
    let alphabet = [
        '*',
        '_',
        '|',
        '`',
        '[',
        ']',
        '(',
        ')',
        '<',
        '>',
        '\\',
        '\n',
        ' ',
        '#',
        '-',
        '~',
        '@',
        ':',
        '/',
        '1',
        'a',
        '\u{1F600}',
        '\u{200D}',
        '\u{FE0F}',
        '\0',
    ];
    let mut seed = 0x5eed_u64;
    for iteration in 0..3000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let len = (seed >> 32) as usize % 384;
        let mut bytes = Vec::with_capacity(len);
        let mut text = String::new();
        for _ in 0..len {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            bytes.push((seed >> 32) as u8);
            text.push(alphabet[(seed >> 32) as usize % alphabet.len()]);
        }
        for value in [String::from_utf8_lossy(&bytes).into_owned(), text] {
            let blocks = parse(&value);
            let _ = is_jumbo(&blocks);
            let preview = plain_text(&blocks);
            assert!(!preview.contains('\n'), "iteration {iteration}");
        }
    }
}

#[test]
fn markdown_performance_4000_char_messages() {
    for pattern in [
        "*",
        "_",
        "|",
        "`",
        "[",
        "<",
        "*_ |`[",
        "**a _b |c [",
        "[x](https://",
        "a```b```",
    ] {
        let value: String = pattern.chars().cycle().take(4000).collect();
        let started = Instant::now();
        let _ = parse(&value);
        assert!(
            started.elapsed() < Duration::from_millis(50),
            "pattern {pattern:?} took {:?}",
            started.elapsed()
        );
    }
}
