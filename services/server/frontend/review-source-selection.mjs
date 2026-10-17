// Non-editable CodeMirror content can have a native selection that its state
// does not track (notably after blur, file navigation, and panel scrolling).
export function nativeSourceRange(view, selection = view?.dom.ownerDocument.getSelection()) {
  if (!view || !selection || selection.isCollapsed || !selection.rangeCount
    || !view.contentDOM.contains(selection.anchorNode) || !view.contentDOM.contains(selection.focusNode)) return null;
  const anchor = view.posAtDOM(selection.anchorNode, selection.anchorOffset);
  const head = view.posAtDOM(selection.focusNode, selection.focusOffset);
  const from = Math.min(anchor, head), to = Math.max(anchor, head);
  return from === to ? null : { from, to, empty: false };
}
