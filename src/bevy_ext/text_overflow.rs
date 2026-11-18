pub fn ellipsize<E>(
    text: &str,
    max_width: f32,
    mut measure: impl FnMut(&str) -> Result<f32, E>,
) -> Result<(String, f32), E> {
    let natural_width = measure(text)?;
    if natural_width <= max_width {
        return Ok((text.to_owned(), natural_width));
    }
    for (end, _) in text.char_indices().rev() {
        let candidate = format!("{}\u{2026}", &text[..end]);
        if measure(&candidate)? <= max_width {
            return Ok((candidate, natural_width));
        }
    }
    Ok((String::new(), natural_width))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipsis_uses_measured_width_and_preserves_original_extent() {
        let measure = |text: &str| -> Result<f32, ()> {
            Ok(text
                .chars()
                .map(|value| if value == 'W' { 4.0 } else { 1.0 })
                .sum())
        };
        assert_eq!(ellipsize("abc", 3.0, measure), Ok(("abc".into(), 3.0)));
        assert_eq!(
            ellipsize("WWWabc", 10.0, measure),
            Ok(("WW\u{2026}".into(), 15.0))
        );
        assert_eq!(
            ellipsize("Caf\u{e9}", 3.0, measure),
            Ok(("Ca\u{2026}".into(), 4.0))
        );
        assert_eq!(ellipsize("abc", 0.0, measure), Ok((String::new(), 3.0)));
        assert_eq!(
            ellipsize("abc", 2.0, |_| Err::<f32, _>("Font pending")),
            Err("Font pending")
        );
    }
}
