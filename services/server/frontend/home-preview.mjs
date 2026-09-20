export function successfulPreviewBuild(role, paper, buildPayload) {
  if (role === 'mentor') {
    return paper?.pdf_available ? paper.current_build_id || null : null;
  }
  return buildPayload?.build?.current_build_id || null;
}
