#[test]
fn styled_lines_keep_their_colors_and_drop_other_escapes() {
    use ratatui::style::{Color, Modifier};

    let line = crate::ui::top::ansi::line(
        "\u{1b}[1;36mbuilding\u{1b}[0m \u{1b}[38;5;208mfirefox\u{1b}[39m\u{1b}[K \u{1b}]8;;https://x\u{7}link",
    );
    let spans: Vec<_> = line
        .spans
        .iter()
        .map(|span| (span.content.as_ref(), span.style))
        .collect();

    assert_eq!(spans[0].0, "building");
    assert_eq!(spans[0].1.fg, Some(Color::Cyan));
    assert!(spans[0].1.add_modifier.contains(Modifier::BOLD));
    assert_eq!(spans[1].0, " ");
    assert_eq!(spans[1].1.fg, None);
    assert_eq!(spans[2].0, "firefox");
    assert_eq!(spans[2].1.fg, Some(Color::Indexed(208)));
    assert_eq!(
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>(),
        "building firefox link"
    );
}
