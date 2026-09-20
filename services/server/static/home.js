import * as pdfjsLib from '/static/pdf.min.mjs';
import { successfulPreviewBuild } from '/static/home-preview.mjs';

pdfjsLib.GlobalWorkerOptions.workerSrc = '/static/pdf.worker.min.mjs';

const role = document.body.dataset.role;
const grid = document.getElementById('projectGrid');
const notice = document.getElementById('projectsNotice');
const account = document.getElementById('homeAccount');
const roleLabel = document.getElementById('homeRole');
const logo = document.getElementById('homeLogo');
const openWorkspace = document.getElementById('openWorkspace');
const isMentor = role === 'mentor';

async function api(path) {
  const response = await fetch(path, { credentials: 'same-origin' });
  if (!response.ok) throw new Error((await response.json().catch(() => ({}))).error || `Request failed (${response.status})`);
  return response.json();
}

function text(tag, className, value) {
  const element = document.createElement(tag);
  element.className = className;
  element.textContent = value;
  return element;
}

function formatUpdated(value) {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? 'Updated recently' : `Updated ${date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })}`;
}

function pdfUrl(paper, build) {
  return `/api/v2/papers/${paper.id}/artifacts/pdf?build=${build}`;
}

async function renderPreview(canvas, paper, build) {
  try {
    const pdf = await pdfjsLib.getDocument({ url: pdfUrl(paper, build), withCredentials: true }).promise;
    const page = await pdf.getPage(1);
    const viewport = page.getViewport({ scale: 1 });
    const availableWidth = Math.max(120, canvas.parentElement.clientWidth - 28);
    const scale = Math.min(availableWidth / viewport.width, 1.35);
    const sized = page.getViewport({ scale });
    canvas.width = Math.ceil(sized.width);
    canvas.height = Math.ceil(sized.height);
    await page.render({ canvasContext: canvas.getContext('2d'), viewport: sized }).promise;
    canvas.closest('.project-preview').classList.add('has-preview');
  } catch (_) {
    canvas.closest('.project-preview').replaceChildren(text('span', 'preview-placeholder', 'Preview unavailable'));
  }
}

function createCard(paper) {
  const href = isMentor ? `/review?paper=${paper.id}` : `/write?paper=${paper.id}`;
  const card = document.createElement('a');
  card.className = 'project-card';
  card.href = href;
  card.dataset.projectId = paper.id;
  const preview = document.createElement('div');
  preview.className = 'project-preview';
  const canvas = document.createElement('canvas');
  canvas.setAttribute('aria-hidden', 'true');
  preview.append(canvas, text('span', 'preview-placeholder', 'No successful PDF yet'));
  const body = document.createElement('div');
  body.className = 'project-card-body';
  body.append(text('h2', 'project-card-title', paper.name), text('p', 'project-card-meta', formatUpdated(paper.updated_at || paper.latest_activity)));
  if (isMentor) {
    const openReviews = Number(paper.open_review_count || 0);
    body.append(text('span', 'project-status review-status', openReviews ? `${openReviews} open review${openReviews === 1 ? '' : 's'}` : 'No active review'));
    body.append(text('span', 'project-card-action', 'Open Review'));
  }
  else if (paper.status) body.append(text('span', `project-status status-${paper.status.toLowerCase()}`, paper.status.replaceAll('_', ' ')));
  card.append(preview, body);
  return { card, canvas, preview };
}

async function loadPreview(item, paper) {
  const buildPayload = isMentor ? paper : await api(`/api/v2/papers/${paper.id}/builds`);
  const build = successfulPreviewBuild(role, paper, buildPayload);
  if (!build || (isMentor && !paper.pdf_available)) {
    item.canvas.remove();
    return;
  }
  await renderPreview(item.canvas, paper, build);
}

async function load() {
  const [identity, papersPayload] = await Promise.all([
    api('/api/v2/me'),
    isMentor ? api('/api/v2/mentor/papers') : api('/api/v2/writer/papers'),
  ]);
  account.textContent = identity.display_name || identity.email;
  roleLabel.textContent = identity.role;
  if (identity.branding_logo_url) { logo.src = identity.branding_logo_url; logo.hidden = false; }
  openWorkspace.href = isMentor ? '/review' : '/write';
  const papers = isMentor ? papersPayload.papers : papersPayload;
  if (!papers.length) {
    notice.textContent = isMentor ? 'No reports are assigned for review yet.' : 'No projects yet. Open Writer to create your first paper.';
    return;
  }
  notice.textContent = isMentor
    ? `${papers.length} assigned report${papers.length === 1 ? '' : 's'}`
    : `${papers.length} project${papers.length === 1 ? '' : 's'}`;
  const items = papers.map((paper) => { const item = createCard(paper); grid.append(item.card); return { item, paper }; });
  await Promise.all(items.map(({ item, paper }) => loadPreview(item, paper)));
}

load().catch((error) => { notice.textContent = error.message; notice.classList.add('is-error'); });
