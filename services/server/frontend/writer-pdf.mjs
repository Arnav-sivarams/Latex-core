export function pdfPreviewState(build) {
  const viewer = Boolean(build.current_build_id);
  return {
    empty: !viewer,
    viewer,
    rebuildingWithLastGood: viewer && Boolean(build.active_build_id),
    failedWithLastGood: viewer && build.latest_status === 'failed',
  };
}
