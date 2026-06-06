use super::target;

#[derive(Debug)]
pub struct Decls<'a>(pub Vec<target::Target<'a>>);

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Multiple targets match")]
    Ambiguous,
    #[error("Target not found")]
    NotFound,
}

impl<'a> Decls<'a> {
    pub fn find_target_by_string(
        &self,
        s: &str,
    ) -> Result<&target::Target<'a>, Error> {
        let candidates: Vec<_> =
            self.0.iter().filter(|t| t.matches_string(s)).collect();
        match candidates.len() {
            l if l == 0 => Err(Error::NotFound),
            l if l > 1 => Err(Error::Ambiguous),
            _ => Ok(candidates[0]),
        }
    }
}
