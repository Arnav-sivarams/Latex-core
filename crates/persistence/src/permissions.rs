//! Central, validated authorization vocabulary and deterministic resolver.
//!
//! Route handlers ask this module for a capability; they never interpret role
//! booleans themselves. Database loading is intentionally separate so this
//! resolver remains easy to audit and unit test.

use crate::AccountType;
use std::collections::BTreeSet;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Permission {
    ProjectRead,
    ProjectCreate,
    ProjectManage,
    ProjectDelete,
    FileRead,
    FileWrite,
    FileCreate,
    FileUpload,
    FileRename,
    FileDelete,
    FileSetMain,
    FileProtectedWrite,
    CompileSubmit,
    ArtifactRead,
    TeamCreate,
    TeamMembersManage,
    TeamProjectManage,
    MentorGroupCreate,
    ReviewRead,
    ReviewComment,
    ReviewResolve,
    ReviewManage,
    TemplateUse,
    TemplateManage,
    AdminUsers,
    AdminTeams,
    AdminProjects,
    AdminTemplates,
    AdminJobs,
    AdminStorage,
    AdminBackups,
    AdminAudit,
    AdminSecurity,
    AdminSystem,
}

impl Permission {
    pub const ALL: [Self; 34] = [
        Self::ProjectRead,
        Self::ProjectCreate,
        Self::ProjectManage,
        Self::ProjectDelete,
        Self::FileRead,
        Self::FileWrite,
        Self::FileCreate,
        Self::FileUpload,
        Self::FileRename,
        Self::FileDelete,
        Self::FileSetMain,
        Self::FileProtectedWrite,
        Self::CompileSubmit,
        Self::ArtifactRead,
        Self::TeamCreate,
        Self::TeamMembersManage,
        Self::TeamProjectManage,
        Self::MentorGroupCreate,
        Self::ReviewRead,
        Self::ReviewComment,
        Self::ReviewResolve,
        Self::ReviewManage,
        Self::TemplateUse,
        Self::TemplateManage,
        Self::AdminUsers,
        Self::AdminTeams,
        Self::AdminProjects,
        Self::AdminTemplates,
        Self::AdminJobs,
        Self::AdminStorage,
        Self::AdminBackups,
        Self::AdminAudit,
        Self::AdminSecurity,
        Self::AdminSystem,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProjectRead => "project.read",
            Self::ProjectCreate => "project.create",
            Self::ProjectManage => "project.manage",
            Self::ProjectDelete => "project.delete",
            Self::FileRead => "file.read",
            Self::FileWrite => "file.write",
            Self::FileCreate => "file.create",
            Self::FileUpload => "file.upload",
            Self::FileRename => "file.rename",
            Self::FileDelete => "file.delete",
            Self::FileSetMain => "file.set_main",
            Self::FileProtectedWrite => "file.protected.write",
            Self::CompileSubmit => "compile.submit",
            Self::ArtifactRead => "artifact.read",
            Self::TeamCreate => "team.create",
            Self::TeamMembersManage => "team.members.manage",
            Self::TeamProjectManage => "team.project.manage",
            Self::MentorGroupCreate => "mentor_group.create",
            Self::ReviewRead => "review.read",
            Self::ReviewComment => "review.comment",
            Self::ReviewResolve => "review.resolve",
            Self::ReviewManage => "review.manage",
            Self::TemplateUse => "template.use",
            Self::TemplateManage => "template.manage",
            Self::AdminUsers => "admin.users",
            Self::AdminTeams => "admin.teams",
            Self::AdminProjects => "admin.projects",
            Self::AdminTemplates => "admin.templates",
            Self::AdminJobs => "admin.jobs",
            Self::AdminStorage => "admin.storage",
            Self::AdminBackups => "admin.backups",
            Self::AdminAudit => "admin.audit",
            Self::AdminSecurity => "admin.security",
            Self::AdminSystem => "admin.system",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|permission| permission.as_str() == value)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OverrideEffect {
    Allow,
    Deny,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct ProjectRoles {
    pub writer: bool,
    pub mentor: bool,
    pub project_manager: bool,
}
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct GroupRoles {
    pub group_manager: bool,
}

#[derive(Clone, Debug, Default)]
pub struct PermissionResolver {
    granted: BTreeSet<Permission>,
    denied: BTreeSet<Permission>,
}

impl PermissionResolver {
    #[must_use]
    pub fn new(
        account: AccountType,
        group: GroupRoles,
        project: ProjectRoles,
        overrides: impl IntoIterator<Item = (Permission, OverrideEffect)>,
    ) -> Self {
        let mut resolver = Self::default();
        resolver.add_account(account);
        resolver.add_group(group);
        resolver.add_project(project);
        for (permission, effect) in overrides {
            match effect {
                OverrideEffect::Allow => {
                    resolver.granted.insert(permission);
                }
                OverrideEffect::Deny => {
                    resolver.denied.insert(permission);
                }
            }
        }
        resolver
    }

    #[must_use]
    pub fn allows(&self, permission: Permission) -> bool {
        !self.denied.contains(&permission) && self.granted.contains(&permission)
    }
    #[must_use]
    pub fn capabilities(&self) -> Vec<&'static str> {
        Permission::ALL
            .into_iter()
            .filter(|p| self.allows(*p))
            .map(Permission::as_str)
            .collect()
    }
    #[must_use]
    pub fn may_delegate(&self, permission: Permission) -> bool {
        self.allows(permission)
    }
    #[must_use]
    pub fn may_delegate_role(&self, role: ProjectRoles) -> bool {
        let writer = [
            Permission::ProjectRead,
            Permission::FileRead,
            Permission::FileWrite,
            Permission::FileCreate,
            Permission::FileUpload,
            Permission::FileRename,
            Permission::FileDelete,
            Permission::FileSetMain,
            Permission::CompileSubmit,
            Permission::ArtifactRead,
        ];
        let mentor = [
            Permission::ProjectRead,
            Permission::FileRead,
            Permission::CompileSubmit,
            Permission::ArtifactRead,
            Permission::ReviewRead,
            Permission::ReviewComment,
            Permission::ReviewManage,
        ];
        let manager = [
            Permission::ProjectRead,
            Permission::ProjectManage,
            Permission::TeamProjectManage,
        ];
        (!role.writer
            || writer
                .into_iter()
                .all(|permission| self.may_delegate(permission)))
            && (!role.mentor
                || mentor
                    .into_iter()
                    .all(|permission| self.may_delegate(permission)))
            && (!role.project_manager
                || manager
                    .into_iter()
                    .all(|permission| self.may_delegate(permission)))
    }

    fn grant(&mut self, permissions: &[Permission]) {
        self.granted.extend(permissions.iter().copied());
    }
    fn add_account(&mut self, account: AccountType) {
        self.grant(&[
            Permission::ProjectCreate,
            Permission::TemplateUse,
            Permission::TeamCreate,
        ]);
        if matches!(account, AccountType::Professor | AccountType::Admin) {
            self.grant(&[Permission::MentorGroupCreate]);
        }
        if account.is_admin() {
            self.grant(&Permission::ALL);
        }
    }
    fn add_group(&mut self, group: GroupRoles) {
        if group.group_manager {
            self.grant(&[
                Permission::TeamMembersManage,
                Permission::TeamProjectManage,
                Permission::ProjectCreate,
            ]);
        }
    }
    fn add_project(&mut self, project: ProjectRoles) {
        if project.writer {
            self.grant(&[
                Permission::ProjectRead,
                Permission::FileRead,
                Permission::FileWrite,
                Permission::FileCreate,
                Permission::FileUpload,
                Permission::FileRename,
                Permission::FileDelete,
                Permission::FileSetMain,
                Permission::CompileSubmit,
                Permission::ArtifactRead,
            ]);
        }
        if project.mentor {
            self.grant(&[
                Permission::ProjectRead,
                Permission::FileRead,
                Permission::CompileSubmit,
                Permission::ArtifactRead,
                Permission::ReviewRead,
                Permission::ReviewComment,
                Permission::ReviewManage,
            ]);
        }
        if project.project_manager {
            self.grant(&[
                Permission::ProjectRead,
                Permission::ProjectManage,
                Permission::TeamProjectManage,
            ]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_roles_are_composable_and_distinct_from_institutional_type() {
        let mentor = PermissionResolver::new(
            AccountType::Professor,
            GroupRoles::default(),
            ProjectRoles {
                mentor: true,
                ..ProjectRoles::default()
            },
            [],
        );
        assert!(mentor.allows(Permission::CompileSubmit));
        assert!(!mentor.allows(Permission::FileWrite));
        let writer_mentor = PermissionResolver::new(
            AccountType::Student,
            GroupRoles::default(),
            ProjectRoles {
                writer: true,
                mentor: true,
                project_manager: false,
            },
            [],
        );
        assert!(writer_mentor.allows(Permission::FileWrite));
        assert!(writer_mentor.allows(Permission::ReviewComment));
    }
    #[test]
    fn deny_wins_over_allow() {
        let resolver = PermissionResolver::new(
            AccountType::Student,
            GroupRoles::default(),
            ProjectRoles {
                writer: true,
                ..ProjectRoles::default()
            },
            [(Permission::FileWrite, OverrideEffect::Deny)],
        );
        assert!(!resolver.allows(Permission::FileWrite));
    }
    #[test]
    fn deny_wins_for_release_critical_capabilities() {
        let student = PermissionResolver::new(
            AccountType::Student,
            GroupRoles::default(),
            ProjectRoles::default(),
            [(Permission::TeamCreate, OverrideEffect::Deny)],
        );
        assert!(!student.allows(Permission::TeamCreate));
        let professor = PermissionResolver::new(
            AccountType::Professor,
            GroupRoles::default(),
            ProjectRoles::default(),
            [(Permission::MentorGroupCreate, OverrideEffect::Deny)],
        );
        assert!(!professor.allows(Permission::MentorGroupCreate));
        let writer = PermissionResolver::new(
            AccountType::Student,
            GroupRoles::default(),
            ProjectRoles {
                writer: true,
                ..ProjectRoles::default()
            },
            [
                (Permission::CompileSubmit, OverrideEffect::Deny),
                (Permission::FileDelete, OverrideEffect::Deny),
            ],
        );
        assert!(!writer.allows(Permission::CompileSubmit));
        assert!(!writer.allows(Permission::FileDelete));
        let mentor = PermissionResolver::new(
            AccountType::Professor,
            GroupRoles::default(),
            ProjectRoles {
                mentor: true,
                ..ProjectRoles::default()
            },
            [(Permission::ArtifactRead, OverrideEffect::Deny)],
        );
        assert!(!mentor.allows(Permission::ArtifactRead));
    }
    #[test]
    fn delegation_never_exceeds_effective_capability() {
        let resolver = PermissionResolver::new(
            AccountType::Student,
            GroupRoles {
                group_manager: true,
            },
            ProjectRoles::default(),
            [],
        );
        assert!(!resolver.may_delegate(Permission::FileProtectedWrite));
        assert!(resolver.may_delegate(Permission::TeamMembersManage));
    }
}
