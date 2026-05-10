#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConversionMode {
    Single,
    Crawl,
}

pub fn boundary_name() -> &'static str {
    "converter"
}

#[cfg(test)]
mod tests {
    use super::{ConversionMode, boundary_name};

    #[test]
    fn exposes_converter_boundary_name() {
        assert_eq!(boundary_name(), "converter");
    }

    #[test]
    fn declares_single_and_crawl_modes() {
        assert_ne!(ConversionMode::Single, ConversionMode::Crawl);
    }
}
