//! Repository operations retain lock ownership across read, validation and commit.

use work_model::discussion::DiscussionSession;
use work_operations::discussion::DiscussionOperation;

use crate::error::WorkError;

pub trait DiscussionRepository {
    fn read_current(&self, requirement_id: &str) -> Result<Option<DiscussionSession>, WorkError>;
    fn read_history(
        &self,
        requirement_id: &str,
        revision: u64,
    ) -> Result<DiscussionSession, WorkError>;
    fn initialize(
        &self,
        session: &DiscussionSession,
        recover: bool,
    ) -> Result<DiscussionSession, WorkError>;
    fn submit(
        &self,
        requirement_id: &str,
        operation: &DiscussionOperation,
        recover: bool,
    ) -> Result<DiscussionSession, WorkError>;
}
