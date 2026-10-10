// CodeMirror's basic setup, with Yjs as the sole undo/redo owner.
import { lineNumbers, highlightActiveLineGutter, highlightSpecialChars, drawSelection, dropCursor, rectangularSelection, crosshairCursor, highlightActiveLine, keymap } from '@codemirror/view';
import { EditorState, Prec } from '@codemirror/state';
import { foldGutter, indentOnInput, syntaxHighlighting, defaultHighlightStyle, bracketMatching, foldKeymap } from '@codemirror/language';
import { defaultKeymap } from '@codemirror/commands';
import { highlightSelectionMatches, searchKeymap } from '@codemirror/search';
import { closeBrackets, autocompletion, closeBracketsKeymap, completionKeymap } from '@codemirror/autocomplete';
import { lintKeymap } from '@codemirror/lint';
import { yUndoManagerKeymap } from 'y-codemirror.next';

export const writerEditorSetup = [
  lineNumbers(), highlightActiveLineGutter(), highlightSpecialChars(), foldGutter(),
  drawSelection(), dropCursor(), EditorState.allowMultipleSelections.of(true),
  indentOnInput(), syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
  bracketMatching(), closeBrackets(), autocompletion(), rectangularSelection(),
  crosshairCursor(), highlightActiveLine(), highlightSelectionMatches(),
  Prec.highest(keymap.of(yUndoManagerKeymap)),
  keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...searchKeymap, ...foldKeymap, ...completionKeymap, ...lintKeymap]),
];
