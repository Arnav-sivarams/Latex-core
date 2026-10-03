export function pdfPreviewState(build) {
  const viewer = Boolean(build.current_build_id);
  return {
    empty: !viewer,
    viewer,
    rebuildingWithLastGood: viewer && Boolean(build.active_build_id),
    failedWithLastGood: viewer && build.latest_status === 'failed',
  };
}

export function pdfPointFromClient(clientX, clientY, bounds, zoomPercent) {
  const scale = Math.max(0.01, (Number(zoomPercent) || 100) / 100);
  return {
    x: Math.max(0, (Number(clientX) - Number(bounds.left)) / scale),
    y: Math.max(0, (Number(clientY) - Number(bounds.top)) / scale),
  };
}
