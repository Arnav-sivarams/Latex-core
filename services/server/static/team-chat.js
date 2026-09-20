const teamsElement = document.getElementById('chatTeams');
const messagesElement = document.getElementById('chatMessages');
const titleElement = document.getElementById('chatTitle');
const accountElement = document.getElementById('chatAccount');
const roleElement = document.getElementById('chatRole');
const form = document.getElementById('chatForm');
const body = document.getElementById('chatBody');
const send = form.querySelector('button');
const notice = document.getElementById('chatNotice');
let selectedTeam = null;
let poll = null;

async function request(path, options = {}) {
  const response = await fetch(path, { credentials: 'same-origin', ...options });
  if (!response.ok) throw new Error((await response.json().catch(() => ({}))).error || `Request failed (${response.status})`);
  return response.status === 204 ? null : response.json();
}
function element(tag, className, value) { const node = document.createElement(tag); node.className = className; node.textContent = value; return node; }
function formatTime(value) { const date = new Date(value); return Number.isNaN(date.valueOf()) ? '' : date.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' }); }
function renderMessages(messages) {
  messagesElement.replaceChildren();
  if (!messages.length) { messagesElement.append(element('p', 'empty-copy', 'No messages yet. Start the conversation.')); return; }
  for (const message of messages) {
    const article = document.createElement('article'); article.className = 'team-chat-message';
    const meta = document.createElement('header'); meta.append(element('strong', '', message.author_name), element('time', '', formatTime(message.created_at)));
    const messageBody = document.createElement('p');
    messageBody.textContent = message.body;
    article.append(meta, messageBody); messagesElement.append(article);
  }
  messagesElement.scrollTop = messagesElement.scrollHeight;
}
async function loadMessages() {
  if (!selectedTeam) return;
  try { renderMessages(await request(`/api/v2/team-chats/${selectedTeam.id}/messages`)); notice.textContent = ''; }
  catch (error) { notice.textContent = error.message; }
}
function selectTeam(team) {
  selectedTeam = team; titleElement.textContent = team.name; body.disabled = false; send.disabled = false;
  [...teamsElement.querySelectorAll('button')].forEach((button) => button.setAttribute('aria-current', String(button.dataset.teamId === team.id)));
  loadMessages(); window.clearInterval(poll); poll = window.setInterval(loadMessages, 4000);
}
function renderTeams(teams) {
  teamsElement.replaceChildren();
  if (!teams.length) { teamsElement.append(element('p', 'empty-copy', 'You are not assigned to a Team chat.')); return; }
  for (const team of teams) { const button = element('button', 'team-chat-team', team.name); button.type = 'button'; button.dataset.teamId = team.id; button.addEventListener('click', () => selectTeam(team)); teamsElement.append(button); }
  selectTeam(teams[0]);
}
form.addEventListener('submit', async (event) => {
  event.preventDefault(); if (!selectedTeam || !body.value.trim()) return;
  send.disabled = true;
  try { await request(`/api/v2/team-chats/${selectedTeam.id}/messages`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ body: body.value }) }); body.value = ''; await loadMessages(); }
  catch (error) { notice.textContent = error.message; }
  finally { send.disabled = false; body.focus(); }
});
Promise.all([request('/api/v2/me'), request('/api/v2/team-chats')]).then(([identity, teams]) => { accountElement.textContent = identity.display_name || identity.email; roleElement.textContent = identity.role; renderTeams(teams); }).catch((error) => { notice.textContent = error.message; });
