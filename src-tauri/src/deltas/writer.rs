use anyhow::{Context, Result};

use crate::{
    gb_repository,
    writer::{self, Writer},
};

use super::Delta;

pub struct DeltasWriter<'writer> {
    repository: &'writer gb_repository::Repository,
    writer: writer::DirWriter,
}

impl<'writer> DeltasWriter<'writer> {
    pub fn new(repository: &'writer gb_repository::Repository) -> Result<Self> {
        let writer = writer::DirWriter::open(repository.root());
        repository
            .get_or_create_current_session()
            .context("failed to create session")?;
        Ok(Self { writer, repository })
    }

    pub fn write<P: AsRef<std::path::Path>>(&self, path: P, deltas: &Vec<Delta>) -> Result<()> {
        self.repository.lock()?;
        defer! {
            self.repository.unlock().unwrap();
        }

        let path = path.as_ref();
        let raw_deltas = serde_json::to_string(&deltas)?;

        self.writer.write_string(
            self.repository.deltas_path().join(path).to_str().unwrap(),
            &raw_deltas,
        )?;

        log::info!(
            "{}: wrote deltas for {}",
            self.repository.project_id,
            path.display()
        );

        Ok(())
    }
}
