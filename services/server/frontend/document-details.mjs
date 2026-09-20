export function frontMatterStatus(detail) {
  const missing = detail.missing_required_fields?.length || 0;
  const warnings = detail.warnings?.length || 0;
  if (missing) return `Front Matter needs ${missing} details`;
  if (warnings) return `Front Matter has ${warnings} template warnings`;
  return detail.status === 'READY' ? 'Front Matter ready' : `Front Matter: ${detail.status.replaceAll('_', ' ')}`;
}

export function editableDetail(detail, field) {
  const teamMetadata = field.source === 'team.semester' || field.source === 'team.academic_year';
  return Boolean(detail.can_edit && (field.editable ?? (!field.source || field.allow_team_override || teamMetadata)));
}

export function needsFirstUseDetails(detail) {
  return Boolean(detail.can_edit && (!detail.project_metadata?.setup_complete || ((detail.pack_id || detail.single_source) && detail.missing_required_fields?.length)));
}
