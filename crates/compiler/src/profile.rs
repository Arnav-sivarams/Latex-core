use crate::CompilerError;
use core_types::{LatexmkProfileId, ShellPolicy};

pub fn profile_for(policy: ShellPolicy) -> Result<LatexmkProfileId, CompilerError> {
    let value = match policy {
        ShellPolicy::Safe => "safe-v1",
        ShellPolicy::Restricted => "restricted-v1",
        ShellPolicy::Compatibility => return Err(CompilerError::UnsupportedShellPolicy { policy }),
    };
    LatexmkProfileId::parse(value).map_err(|error| CompilerError::InternalInvariant {
        message: error.to_string(),
    })
}
