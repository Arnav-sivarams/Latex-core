import * as Y from 'yjs';

export function normalizeRectangle(rectangle, width, height) {
  if (!(width > 0) || !(height > 0)) throw new Error('page dimensions must be positive');
  const x1 = Math.max(0, Math.min(rectangle.x1, rectangle.x2, width));
  const y1 = Math.max(0, Math.min(rectangle.y1, rectangle.y2, height));
  const x2 = Math.max(0, Math.min(Math.max(rectangle.x1, rectangle.x2), width));
  const y2 = Math.max(0, Math.min(Math.max(rectangle.y1, rectangle.y2), height));
  return { x: x1 / width, y: y1 / height, width: (x2 - x1) / width, height: (y2 - y1) / height };
}

export function denormalizeRectangle(rectangle, width, height) {
  return { x: rectangle.x * width, y: rectangle.y * height, width: rectangle.width * width, height: rectangle.height * height };
}

export function resolveSuggestionRange(doc, ytext, encodedStart, encodedEnd) {
  if (!encodedStart || !encodedEnd) return null;
  try {
    const start = Y.createAbsolutePositionFromRelativePosition(Y.decodeRelativePosition(encodedStart), doc);
    const end = Y.createAbsolutePositionFromRelativePosition(Y.decodeRelativePosition(encodedEnd), doc);
    if (!start || !end || start.type !== ytext || end.type !== ytext || end.index < start.index) return null;
    return { from: start.index, to: end.index };
  } catch {
    return null;
  }
}
