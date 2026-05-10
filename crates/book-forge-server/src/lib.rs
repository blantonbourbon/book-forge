pub fn boundary_name() -> &'static str {
    "backend"
}

#[cfg(test)]
mod tests {
    use super::boundary_name;

    #[test]
    fn exposes_backend_boundary_name() {
        assert_eq!(boundary_name(), "backend");
    }
}
