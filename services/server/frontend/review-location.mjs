// Plain-language location and status of a review comment, shared by the Writer and Mentor views.
export function reviewLocation(thread, files = []) {
  const source = thread?.source_anchor || null;
  const pdf = thread?.pdf_anchor || null;
  const mapped = pdf?.mapped_file_id ? files.find((file) => file.file_id === pdf.mapped_file_id) : null;
  const path = source?.path || mapped?.path || '';
  const start = source?.start_line || (!source && pdf?.mapped_line) || null;
  const end = source?.end_line && source.end_line !== start ? source.end_line : null;
  const lines = start ? `${end ? 'lines' : 'line'} ${start}${end ? `–${end}` : ''}` : '';
  const page = pdf?.page ? `PDF page ${pdf.page}` : '';
  const file = path ? path.split('/').at(-1) : '';
  const label = [file || (page ? '' : 'Location not recorded'), lines, page].filter(Boolean).join(' · ');
  return { path, file, lines, page, label };
}

export function reviewStateLabel(state, role = 'writer') {
  if (state === 'ADDRESSED') return role === 'mentor' ? 'Writer marked done — confirm or reopen' : 'Done — awaiting Mentor confirmation';
  return { OPEN: 'Open', REOPENED: 'Reopened', RESOLVED: 'Resolved' }[state] || state;
}
