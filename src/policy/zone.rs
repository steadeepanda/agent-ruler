use crate::model::{ActionKind, Zone};
use crate::utils::is_subpath;

use super::{helpers::owner_uid, PolicyEngine};

impl PolicyEngine {
    // Zone precedence is intentional: secrets > system-critical > shared > workspace > user-data.
    // This enforces the strongest deterministic policy when path patterns overlap.
    pub(super) fn classify_uncached(&self, path: &std::path::Path, kind: ActionKind) -> Zone {
        if self.matches_any(path, &self.compiled.secrets) {
            return Zone::Secrets;
        }

        if self.matches_any(path, &self.compiled.system_critical) {
            return Zone::SystemCritical;
        }

        if self.matches_any(path, &self.compiled.shared) {
            return Zone::Shared;
        }

        if self.matches_any(path, &self.compiled.workspace) || is_subpath(path, &self.workspace) {
            return Zone::Workspace;
        }

        if self.matches_any(path, &self.compiled.user_data) {
            return Zone::UserData;
        }

        #[cfg(target_family = "unix")]
        {
            if let Some(uid) = owner_uid(path) {
                if uid == 0 {
                    if matches!(
                        kind,
                        ActionKind::FileWrite
                            | ActionKind::FileDelete
                            | ActionKind::FileRename
                            | ActionKind::Persistence
                    ) {
                        return Zone::SystemCritical;
                    }
                    return Zone::Shared;
                }
            }
        }

        Zone::UserData
    }
}
