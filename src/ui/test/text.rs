use std::time::Duration;

use colored::Colorize;

use crate::{
    common::model::Timestamp,
    nix::strip_ansi,
    ui::{format, text},
};

#[test]
fn widths_ignore_colors_and_count_wide_glyphs() {
    let colored = format!("{} {}", "✓".green(), "altar".magenta().bold());
    assert_eq!(text::width(&colored), 7);
    assert_eq!(text::width("界"), 2);
}

#[test]
fn fitting_cuts_by_columns_and_keeps_colors_closed() {
    let line = format!("{} building firefox", "altar".magenta());
    let fitted = text::fit(&line, 10);

    assert_eq!(strip_ansi(&fitted), "altar bui…");
    assert_eq!(text::width(&fitted), 10);
    assert!(fitted.ends_with("\u{1b}[0m"));
    assert_eq!(text::fit("short", 10), "short");
}

#[test]
fn padding_counts_columns_not_bytes() {
    let padded = text::pad(&"✓".green().to_string(), 3);
    assert_eq!(strip_ansi(&padded), "✓  ");
    assert_eq!(text::pad_left("7%", 4), "  7%");
}

#[test]
fn durations_and_ages_read_naturally() {
    assert_eq!(format::duration(Duration::from_secs(42)), "42s");
    assert_eq!(format::duration(Duration::from_secs(192)), "3m12s");
    assert_eq!(format::duration(Duration::from_secs(3_840)), "1h04m");

    let now = Timestamp(10_000_000);
    assert_eq!(format::ago(Timestamp(now.0 - 2_000), now), "just now");
    assert_eq!(format::ago(Timestamp(now.0 - 180_000), now), "3m ago");
    assert_eq!(format::ago(Timestamp(now.0 - 7_200_000), now), "2h ago");
}

#[test]
fn byte_counts_share_the_unit_of_the_total() {
    assert_eq!(format::bytes(512), "512 B");
    assert_eq!(format::bytes(3 * 1024 * 1024), "3.0 MiB");
    assert_eq!(
        format::bytes_of(1_288_490_189, 3_650_722_201),
        "1.2/3.4 GiB"
    );
    assert_eq!(format::size_delta(-300 * 1024), "-300.0 KiB");
    assert_eq!(format::size_delta(0), "0 B");
    assert_eq!(format::size_delta(1024), "+1.0 KiB");
}

#[test]
fn derivation_names_lose_their_hash_and_suffix() {
    assert_eq!(
        format::derivation("/nix/store/h0myp2wfggr0x8pfacciv8sfhxyb845d-firefox-121.0.drv"),
        "firefox-121.0"
    );
}
