import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const writerHtml = readFileSync(new URL('../src/write.html', import.meta.url), 'utf8');
const mentorHtml = readFileSync(new URL('../src/review.html', import.meta.url), 'utf8');

function assertControls(html, ids) {
  for (const id of ids) assert.match(html, new RegExp(`id="${id}"`), `${id} must remain in the workspace DOM`);
}

test('Writer visual refresh preserves the complete working surface', () => {
  assertControls(writerHtml, [
    'projectSearch', 'saveFile', 'undoText', 'redoText', 'quickOpen', 'commandPalette',
    'compilePaper', 'sendReview', 'endReview', 'insertMenu', 'problemsToggle',
    'historyToggle', 'commentsToggle', 'documentDetails', 'editorSettings', 'moreActions',
    'newPaper', 'myPapers', 'teamPapers', 'newFile', 'uploadImage', 'fileActionsToggle',
    'fileTree', 'editorMount', 'pdfPage', 'pdfZoom', 'locateInPdf', 'downloadPdf',
    'pdfViewport', 'buildProblemsTab', 'buildLogTab', 'buildLogText', 'workspaceDrawer',
    'renameFile', 'deleteFile', 'structuralUndo', 'structuralRedo',
  ]);
  assert.match(writerHtml, /id="compilePaper" class="[^"]*(?:ui-button-primary|primary)/);
  assert.doesNotMatch(writerHtml, /id="(?:downloadSource|setMain)"/);
  assert.match(writerHtml, /id="mainBadge"/);
});

test('Mentor visual refresh preserves review, locking, source, PDF, and export controls', () => {
  assertControls(mentorHtml, [
    'assignedPapers', 'reviewFiles', 'compileReview', 'reviewGateBadge', 'reviewSelection',
    'mentorCommentsToggle', 'csvExport', 'printExport', 'pushReview', 'draftStatus',
    'editorSettings', 'reviewEditor', 'previousPage', 'nextPage', 'zoomOut', 'zoomIn',
    'pdfCanvas', 'pdfOverlay', 'reviewPopover', 'threadMessage', 'createComment',
    'createSuggestion', 'cancelAnnotation', 'mentorReviewDrawer', 'threadFilters', 'roundList',
  ]);
  assert.match(mentorHtml, /id="compileReview" class="[^"]*(?:ui-button-primary|primary)/);
});
