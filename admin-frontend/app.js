const cfg = window.APP_CONFIG || {};
let idToken = null;
let debugEmail = null;

function showApp() {
  document.getElementById('auth').style.display = 'none';
  document.getElementById('app').style.display = 'block';
  loadLinks();
  loadGroups();
}

window.onGoogleSignIn = ({ credential }) => {
  idToken = credential;
  showApp();
};

function initAuth() {
  const authDisabled = !!cfg.AUTH_DISABLED;
  const g = document.getElementById('auth-google');
  const d = document.getElementById('auth-debug');
  if (authDisabled) {
    g.style.display = 'none';
    d.style.display = 'flex';
    document.getElementById('useDebug').onclick = () => {
      const val = document.getElementById('debugEmail').value.trim();
      if (!val) { alert('Enter a debug email'); return; }
      debugEmail = val;
      showApp();
    };
  } else {
    d.style.display = 'none';
    g.style.display = 'block';
    if (!cfg.GOOGLE_CLIENT_ID) {
      console.warn('GOOGLE_CLIENT_ID missing in config.js');
    }
    window.google?.accounts.id.initialize({ client_id: cfg.GOOGLE_CLIENT_ID, callback: window.onGoogleSignIn, ux_mode: 'popup' });
    window.google?.accounts.id.renderButton(document.querySelector('.g_id_signin'), { theme: 'outline', size: 'large' });
  }
}

async function api(path, opts = {}) {
  const headers = opts.headers || {};
  headers['content-type'] = 'application/json';
  if (idToken) headers['authorization'] = `Bearer ${idToken}`;
  if (debugEmail) headers['X-Debug-User'] = debugEmail;
  const res = await fetch(`${cfg.API_BASE}${path}`, { ...opts, headers });
  let body = null;
  const ct = res.headers.get('content-type') || '';
  if (ct.includes('application/json')) { body = await res.json(); }
  return { ok: res.ok, status: res.status, body };
}

let allLinks = [];
let hasNewFeatures = false;
let currentUser = null;
let currentPage = 0;
let pageSize = 50;
let totalLinks = 0;
let hasMore = false;
let searchDebounce = null;

// Helper to format expiry date for display
function formatExpiry(isoString) {
  if (!isoString) return '-';
  const d = new Date(isoString);
  const now = new Date();
  if (d < now) return 'Expired';
  const diff = d - now;
  const days = Math.floor(diff / (1000 * 60 * 60 * 24));
  if (days <= 0) {
    const hours = Math.floor(diff / (1000 * 60 * 60));
    if (hours <= 0) {
      const mins = Math.floor(diff / (1000 * 60));
      return `${mins}m`;
    }
    return `${hours}h`;
  }
  if (days <= 7) return `${days}d`;
  return d.toLocaleDateString();
}

// Convert ISO string to datetime-local format
function isoToLocal(isoString) {
  if (!isoString) return '';
  const d = new Date(isoString);
  const local = new Date(d.getTime() - d.getTimezoneOffset() * 60000);
  return local.toISOString().slice(0, 16);
}

async function loadLinks() {
  const out = document.querySelector('#listTbl tbody');
  out.innerHTML = '';

  const filterBy = document.getElementById('filterBy').value;
  const filterByGroup = document.getElementById('filterByGroup').value;
  const search = document.getElementById('searchInput').value.trim();

  let url = `/api/links?limit=${pageSize}&offset=${currentPage * pageSize}`;
  if (filterBy) url += `&created_by=${encodeURIComponent(filterBy)}`;
  if (filterByGroup) url += `&group_id=${encodeURIComponent(filterByGroup)}`;
  if (search) url += `&search=${encodeURIComponent(search)}`;

  const r = await api(url);
  if (!r.ok) {
    out.innerHTML = `<tr><td colspan="11">Error ${r.status}</td></tr>`;
    return;
  }

  const links = r.body?.links || [];
  allLinks = links;
  totalLinks = r.body?.total || links.length;
  hasMore = r.body?.has_more || false;

  // Get user info from response
  if (r.body?.user) {
    currentUser = r.body.user;
    updateUserDisplay();
    populateGroupDropdowns();
  }

  // Detect if backend supports new features
  if (links.length > 0) {
    hasNewFeatures = typeof links[0].click_count !== 'undefined';
  }

  updateUIForFeatures();
  updatePagination();

  // Update filter dropdown with unique creators (only if admin)
  if (currentUser?.is_admin && !filterBy && hasNewFeatures) {
    const filterSelect = document.getElementById('filterBy');
    const currentValue = filterSelect.value;
    const creators = [...new Set(links.map(l => l.created_by))].sort();
    filterSelect.innerHTML = '<option value="">All</option>';
    creators.forEach(c => {
      const opt = document.createElement('option');
      opt.value = c;
      opt.textContent = c;
      filterSelect.appendChild(opt);
    });
    filterSelect.value = currentValue;
  }

  for (const l of links) {
    const tr = document.createElement('tr');
    const checkbox = document.createElement('input');
    checkbox.type = 'checkbox';
    checkbox.className = 'link-select';
    checkbox.dataset.slug = l.slug;
    tr.insertCell().append(checkbox);

    const slug = addCell(tr, l.slug + (hasNewFeatures && l.description ? ' *' : ''));
    const shortUrl = addCell(tr, l.short_url);
    const shortUrlHref = httpUrl(l.short_url);
    if (shortUrlHref) {
      const anchor = document.createElement('a');
      anchor.href = shortUrlHref;
      anchor.target = '_blank';
      anchor.rel = 'noreferrer';
      anchor.textContent = l.short_url;
      const copy = actionButton('📋', () => copyToClipboard(l.short_url, copy));
      copy.className = 'copy-btn';
      copy.title = 'Copy';
      shortUrl.replaceChildren(anchor, ' ', copy);
    }

    const original = addCell(tr, l.original_url);
    if (hasNewFeatures) {
      if (l.description) slug.title = l.description;
      original.style.cssText = 'max-width:300px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;';
      original.title = l.original_url;
      addCell(tr, l.click_count);
      addCell(tr, l.is_active ? 'Active' : 'Inactive').style.color = l.is_active ? 'green' : 'red';
      addCell(tr, l.group_id ? (allGroups.find(g => g.id === l.group_id)?.name || l.group_id) : '-');
      const expiry = addCell(tr, formatExpiry(l.expires_at));
      if (l.expires_at) expiry.title = l.expires_at;
      if (expiry.textContent === 'Expired') expiry.style.color = 'red';
    }
    addCell(tr, hasNewFeatures ? new Date(l.created_at).toLocaleDateString() : l.created_at);
    addCell(tr, l.created_by);

    if (hasNewFeatures) {
      const actions = tr.insertCell();
      actions.append(
        actionButton('Edit', () => openEditModal(l.slug)),
        actionButton(l.is_active ? 'Off' : 'On', () => toggleLink(l.slug, l.is_active))
      );
      if (shortUrlHref) actions.append(actionButton('QR', () => showQrCode(l.slug, shortUrlHref)));
    }
    out.appendChild(tr);
  }

  updateSelectedCount();
}

function addCell(row, text) {
  const cell = row.insertCell();
  cell.textContent = text;
  return cell;
}

function actionButton(label, handler) {
  const button = document.createElement('button');
  button.type = 'button';
  button.textContent = label;
  button.addEventListener('click', handler);
  return button;
}

function httpUrl(value) {
  try {
    const url = new URL(value, window.location.href);
    if (url.protocol === 'https:' || url.protocol === 'http:') return url.href;
  } catch {
    return null;
  }
  return null;
}

function roleBadge(role, label = role) {
  const badge = document.createElement('span');
  badge.style.cssText = 'color:white;padding:2px 6px;border-radius:3px;font-size:0.8em;';
  badge.style.backgroundColor = role === 'admin' ? '#4CAF50' : role === 'editor' ? '#2196F3' : '#9e9e9e';
  badge.textContent = label;
  return badge;
}

function updatePagination() {
  const top = document.getElementById('paginationTop');
  const bottom = document.getElementById('paginationBottom');
  const totalEl = document.getElementById('totalLinks');

  if (totalLinks > pageSize) {
    top.style.display = 'flex';
    bottom.style.display = 'flex';
    totalEl.textContent = `${totalLinks} links total`;

    const maxPage = Math.ceil(totalLinks / pageSize) - 1;
    const pageNum = currentPage + 1;

    document.getElementById('pageInfo').textContent = `Page ${pageNum} of ${maxPage + 1}`;
    document.getElementById('pageInfo2').textContent = `Page ${pageNum} of ${maxPage + 1}`;

    document.getElementById('prevPage').disabled = currentPage === 0;
    document.getElementById('prevPage2').disabled = currentPage === 0;
    document.getElementById('nextPage').disabled = !hasMore;
    document.getElementById('nextPage2').disabled = !hasMore;
  } else {
    top.style.display = 'none';
    bottom.style.display = 'none';
  }
}

function updateUserDisplay() {
  const userEl = document.getElementById('userInfo');
  if (userEl && currentUser) {
    const email = document.createElement('strong');
    email.textContent = currentUser.email;
    userEl.replaceChildren('Logged in as: ', email);
    if (currentUser.is_admin) userEl.append(' ', roleBadge('admin', 'Admin'));
  }
}

function updateUIForFeatures() {
  const thead = document.querySelector('#listTbl thead tr');
  const bulkActions = document.getElementById('bulkActions');

  if (hasNewFeatures) {
    const showFilter = currentUser?.is_admin;
    document.getElementById('filterBy').style.display = showFilter ? '' : 'none';
    document.querySelector('label[for="filterBy"]').style.display = showFilter ? '' : 'none';
    bulkActions.style.display = currentUser?.is_admin ? 'flex' : 'none';
    thead.innerHTML = '<th><input type="checkbox" id="selectAll" /></th><th>Slug</th><th>Short URL</th><th>Original URL</th><th>Clicks</th><th>Status</th><th>Group</th><th>Expires</th><th>Created</th><th>By</th><th>Actions</th>';
  } else {
    document.getElementById('filterBy').style.display = 'none';
    document.querySelector('label[for="filterBy"]').style.display = 'none';
    bulkActions.style.display = 'none';
    thead.innerHTML = '<th><input type="checkbox" id="selectAll" /></th><th>Slug</th><th>Short URL</th><th>Original URL</th><th>Created</th><th>By</th>';
  }

  // Re-attach selectAll handler
  document.getElementById('selectAll').onclick = toggleSelectAll;
}

function toggleSelectAll(e) {
  const checked = e.target.checked;
  document.querySelectorAll('.link-select').forEach(cb => cb.checked = checked);
  updateSelectedCount();
}

function getSelectedSlugs() {
  return Array.from(document.querySelectorAll('.link-select:checked')).map(cb => cb.dataset.slug);
}

function updateSelectedCount() {
  const count = getSelectedSlugs().length;
  document.getElementById('selectedCount').textContent = `${count} selected`;
}

function openEditModal(slug) {
  const link = allLinks.find(l => l.slug === slug);
  if (!link) return;

  document.getElementById('editSlug').textContent = slug;
  document.getElementById('editUrl').value = link.original_url;
  document.getElementById('editDesc').value = link.description || '';
  document.getElementById('editActive').checked = link.is_active;
  document.getElementById('editExpires').value = isoToLocal(link.expires_at);
  document.getElementById('editActivateAt').value = isoToLocal(link.activate_at);
  document.getElementById('editRedirectDelay').value = link.redirect_delay || '';
  const editGroup = document.getElementById('editGroup');
  editGroup.dataset.assignedGroup = link.group_id || '';
  populateGroupDropdown('editGroup', link.group_id || '');

  document.getElementById('editModal').style.display = 'block';
  document.getElementById('editModal').dataset.slug = slug;
}

function closeEditModal() {
  document.getElementById('editModal').style.display = 'none';
}

async function saveEdit() {
  const slug = document.getElementById('editModal').dataset.slug;
  const original_url = document.getElementById('editUrl').value.trim();
  const description = document.getElementById('editDesc').value.trim();
  const is_active = document.getElementById('editActive').checked;
  const expiresValue = document.getElementById('editExpires').value;
  const activateAtValue = document.getElementById('editActivateAt').value;
  const redirectDelayValue = document.getElementById('editRedirectDelay').value;
  const groupValue = document.getElementById('editGroup').value;

  if (!original_url) { alert('URL is required'); return; }

  const payload = { original_url, is_active };

  // Handle optional fields
  payload.description = description || null;
  payload.expires_at = expiresValue ? new Date(expiresValue).toISOString() : null;
  payload.activate_at = activateAtValue ? new Date(activateAtValue).toISOString() : null;
  payload.redirect_delay = redirectDelayValue ? parseInt(redirectDelayValue, 10) : null;
  payload.group_id = groupValue || null;

  const r = await api(`/api/links/${slug}`, {
    method: 'PATCH',
    body: JSON.stringify(payload)
  });

  if (r.ok) {
    closeEditModal();
    await loadLinks();
  } else {
    alert(`Error ${r.status}: ${(r.body?.error?.message) || 'failed'}`);
  }
}

async function deleteLink(slug) {
  if (!confirm(`Delete link "${slug}"? This cannot be undone.`)) return;

  const r = await api(`/api/links/${slug}`, { method: 'DELETE' });

  if (r.ok || r.status === 204) {
    closeEditModal();
    await loadLinks();
  } else {
    alert(`Error ${r.status}: ${(r.body?.error?.message) || 'failed'}`);
  }
}

async function toggleLink(slug, currentActive) {
  const r = await api(`/api/links/${slug}`, {
    method: 'PATCH',
    body: JSON.stringify({ is_active: !currentActive })
  });

  if (r.ok) {
    await loadLinks();
  } else {
    alert(`Error ${r.status}: ${(r.body?.error?.message) || 'failed'}`);
  }
}

// Bulk operations
async function bulkDelete() {
  const slugs = getSelectedSlugs();
  if (slugs.length === 0) { alert('Select links first'); return; }
  if (!confirm(`Delete ${slugs.length} links? This cannot be undone.`)) return;

  const r = await api('/api/links/bulk/delete', {
    method: 'POST',
    body: JSON.stringify({ slugs })
  });

  if (r.ok) {
    await loadLinks();
  } else {
    alert(`Error ${r.status}: ${(r.body?.error?.message) || 'failed'}`);
  }
}

async function bulkActivate() {
  const slugs = getSelectedSlugs();
  if (slugs.length === 0) { alert('Select links first'); return; }

  const r = await api('/api/links/bulk/activate', {
    method: 'POST',
    body: JSON.stringify({ slugs })
  });

  if (r.ok) {
    await loadLinks();
  } else {
    alert(`Error ${r.status}: ${(r.body?.error?.message) || 'failed'}`);
  }
}

async function bulkDeactivate() {
  const slugs = getSelectedSlugs();
  if (slugs.length === 0) { alert('Select links first'); return; }

  const r = await api('/api/links/bulk/deactivate', {
    method: 'POST',
    body: JSON.stringify({ slugs })
  });

  if (r.ok) {
    await loadLinks();
  } else {
    alert(`Error ${r.status}: ${(r.body?.error?.message) || 'failed'}`);
  }
}

// QR Code modal functions
let currentQrSlug = '';
let currentQrBaseUrl = '';
let qrRequest;
let qrImageUrl;

function showQrCode(slug, shortUrl) {
  currentQrSlug = slug;
  currentQrBaseUrl = shortUrl;

  const modal = document.getElementById('qrModal');
  const modeSelect = document.getElementById('qrMode');

  // Reset to default mode
  modeSelect.value = '';

  modal.style.display = 'block';
  document.getElementById('qrSlug').textContent = slug;

  // Load QR code with current mode
  loadQrCode();
}

function clearQrImage() {
  qrRequest?.abort();
  if (qrImageUrl) URL.revokeObjectURL(qrImageUrl);
  qrImageUrl = undefined;
  const downloadLink = document.getElementById('qrDownload');
  downloadLink.removeAttribute('href');
  downloadLink.style.display = 'none';
}

async function loadQrCode() {
  clearQrImage();
  const request = new AbortController();
  qrRequest = request;
  const container = document.getElementById('qrContainer');
  const downloadLink = document.getElementById('qrDownload');
  const urlDisplay = document.getElementById('qrUrlDisplay');
  const mode = document.getElementById('qrMode').value;
  const targetUrl = `${currentQrBaseUrl}${mode}`;

  container.textContent = 'Loading QR code...';
  urlDisplay.textContent = `URL: ${targetUrl}`;

  try {
    const response = await fetch(`${targetUrl}.qr`, { signal: request.signal });
    if (!response.ok) throw new Error('Failed to load QR code');
    const svg = await response.text();
    if (request.signal.aborted) return;
    qrImageUrl = URL.createObjectURL(new Blob([svg], { type: 'image/svg+xml' }));
    const image = document.createElement('img');
    image.src = qrImageUrl;
    image.width = 200;
    image.height = 200;
    image.alt = 'QR code';
    container.replaceChildren(image);
    downloadLink.href = qrImageUrl;
    const suffix = mode ? `-${mode.replace('+', 'preview')}` : '';
    downloadLink.download = `${currentQrSlug}${suffix}-qr.svg`;
    downloadLink.style.display = 'inline-block';
  } catch (error) {
    if (!request.signal.aborted) container.textContent = `Error: ${error.message}`;
  }
}

function closeQrModal() {
  clearQrImage();
  document.getElementById('qrModal').style.display = 'none';
  currentQrSlug = '';
  currentQrBaseUrl = '';
}

// Copy to clipboard function
async function copyToClipboard(text, btn) {
  try {
    await navigator.clipboard.writeText(text);
    const orig = btn.textContent;
    btn.textContent = '✓';
    btn.style.color = 'green';
    setTimeout(() => {
      btn.textContent = orig;
      btn.style.color = '';
    }, 1500);
  } catch (err) {
    console.error('Copy failed:', err);
    btn.textContent = '✗';
    btn.style.color = 'red';
    setTimeout(() => {
      btn.textContent = '📋';
      btn.style.color = '';
    }, 1500);
  }
}

// ============================================================================
// Group Management
// ============================================================================

let allGroups = [];
let currentGroupId = null;

// Populate all group dropdowns with current groups
function populateGroupDropdowns() {
  for (const id of ['createGroup', 'editGroup', 'filterByGroup']) {
    populateGroupDropdown(id, document.getElementById(id).value);
  }
}

function populateGroupDropdown(id, value) {
  const el = document.getElementById(id);
  const isFilter = id === 'filterByGroup';
  const assignedGroup = el.dataset.assignedGroup;
  el.replaceChildren(new Option(isFilter ? 'All groups' : 'No group', ''));
  for (const group of allGroups) {
    const canAssign = currentUser?.is_admin || group.role === 'admin' || group.role === 'editor';
    if (isFilter || canAssign || group.id === assignedGroup) {
      el.append(new Option(group.name, group.id));
    }
  }
  if (assignedGroup && !allGroups.some(group => group.id === assignedGroup)) {
    el.append(new Option(assignedGroup, assignedGroup));
  }
  el.value = value;
}

async function loadGroups() {
  const out = document.querySelector('#groupsTbl tbody');
  out.innerHTML = '';

  const r = await api('/api/groups');
  if (!r.ok) {
    out.innerHTML = `<tr><td colspan="5">Error ${r.status}</td></tr>`;
    populateGroupDropdowns(); // Still populate with empty list
    return;
  }

  allGroups = r.body?.groups || [];
  populateGroupDropdowns(); // Update all group dropdowns

  if (allGroups.length === 0) {
    out.innerHTML = '<tr><td colspan="5" class="muted">No groups yet. Create one to organize your links!</td></tr>';
    return;
  }

  for (const g of allGroups) {
    const tr = document.createElement('tr');
    const name = document.createElement('strong');
    name.textContent = g.name;
    tr.insertCell().append(name);
    addCell(tr, g.description || '-').className = 'muted';
    const role = tr.insertCell();
    if (g.role) role.append(roleBadge(g.role));
    addCell(tr, new Date(g.created_at).toLocaleDateString());
    const actions = tr.insertCell();
    actions.append(actionButton('Members', () => openMembersModal(g.id, g.name)));
    if (g.role === 'admin' || currentUser?.is_admin) {
      const remove = actionButton('Delete', () => deleteGroup(g.id));
      remove.style.color = '#dc3545';
      actions.append(actionButton('Edit', () => openGroupEditModal(g.id)), remove);
    }
    out.appendChild(tr);
  }
}

function openGroupCreateModal() {
  document.getElementById('groupModalTitle').textContent = 'Create Group';
  document.getElementById('groupName').value = '';
  document.getElementById('groupDesc').value = '';
  document.getElementById('groupModal').dataset.mode = 'create';
  document.getElementById('groupModal').dataset.groupId = '';
  document.getElementById('groupModal').style.display = 'block';
}

function openGroupEditModal(groupId) {
  const group = allGroups.find(g => g.id === groupId);
  if (!group) return;

  document.getElementById('groupModalTitle').textContent = 'Edit Group';
  document.getElementById('groupName').value = group.name;
  document.getElementById('groupDesc').value = group.description || '';
  document.getElementById('groupModal').dataset.mode = 'edit';
  document.getElementById('groupModal').dataset.groupId = groupId;
  document.getElementById('groupModal').style.display = 'block';
}

function closeGroupModal() {
  document.getElementById('groupModal').style.display = 'none';
}

async function saveGroup() {
  const modal = document.getElementById('groupModal');
  const mode = modal.dataset.mode;
  const groupId = modal.dataset.groupId;
  const name = document.getElementById('groupName').value.trim();
  const description = document.getElementById('groupDesc').value.trim();

  if (!name) { alert('Name is required'); return; }

  const payload = { name };
  if (description) payload.description = description;
  else payload.description = null;

  let r;
  if (mode === 'create') {
    r = await api('/api/groups', { method: 'POST', body: JSON.stringify(payload) });
  } else {
    r = await api(`/api/groups/${groupId}`, { method: 'PATCH', body: JSON.stringify(payload) });
  }

  if (r.ok) {
    closeGroupModal();
    await loadGroups();
  } else {
    alert(`Error ${r.status}: ${(r.body?.error?.message) || 'failed'}`);
  }
}

async function deleteGroup(groupId) {
  if (!confirm('Delete this group? Members will lose access to grouped links.')) return;

  const r = await api(`/api/groups/${groupId}`, { method: 'DELETE' });

  if (r.ok || r.status === 204) {
    await loadGroups();
  } else {
    alert(`Error ${r.status}: ${(r.body?.error?.message) || 'failed'}`);
  }
}

// ============================================================================
// Group Members Management
// ============================================================================

async function openMembersModal(groupId, groupName) {
  currentGroupId = groupId;
  document.getElementById('membersGroupName').textContent = groupName;
  document.getElementById('addMemberEmail').value = '';
  document.getElementById('addMemberRole').value = 'editor';
  document.getElementById('membersModal').style.display = 'block';
  await loadMembers();
}

function closeMembersModal() {
  document.getElementById('membersModal').style.display = 'none';
  currentGroupId = null;
}

async function loadMembers() {
  if (!currentGroupId) return;

  const out = document.querySelector('#membersTbl tbody');
  out.innerHTML = '';

  const r = await api(`/api/groups/${currentGroupId}/members`);
  if (!r.ok) {
    out.innerHTML = `<tr><td colspan="4">Error ${r.status}</td></tr>`;
    return;
  }

  const members = r.body?.members || [];

  if (members.length === 0) {
    out.innerHTML = '<tr><td colspan="4" class="muted">No members yet</td></tr>';
    return;
  }

  // Check if current user can manage
  const group = allGroups.find(g => g.id === currentGroupId);
  const canManage = group?.role === 'admin' || currentUser?.is_admin;

  for (const m of members) {
    const tr = document.createElement('tr');
    addCell(tr, m.email);
    tr.insertCell().append(roleBadge(m.role));
    addCell(tr, new Date(m.added_at).toLocaleDateString());
    const actions = tr.insertCell();
    if (canManage) {
      const remove = actionButton('Remove', () => removeMember(encodeURIComponent(m.email)));
      remove.style.color = '#dc3545';
      actions.append(remove);
    }
    out.appendChild(tr);
  }
}

async function addMember() {
  if (!currentGroupId) return;

  const email = document.getElementById('addMemberEmail').value.trim();
  const role = document.getElementById('addMemberRole').value;

  if (!email) { alert('Email is required'); return; }

  const r = await api(`/api/groups/${currentGroupId}/members`, {
    method: 'POST',
    body: JSON.stringify({ email, role })
  });

  if (r.ok) {
    document.getElementById('addMemberEmail').value = '';
    await loadMembers();
  } else {
    alert(`Error ${r.status}: ${(r.body?.error?.message) || 'failed'}`);
  }
}

async function removeMember(encodedEmail) {
  if (!currentGroupId) return;
  const email = decodeURIComponent(encodedEmail);
  if (!confirm(`Remove ${email} from this group?`)) return;

  const r = await api(`/api/groups/${currentGroupId}/members/${encodedEmail}`, { method: 'DELETE' });

  if (r.ok || r.status === 204) {
    await loadMembers();
  } else {
    alert(`Error ${r.status}: ${(r.body?.error?.message) || 'failed'}`);
  }
}

async function createLink() {
  const original_url = document.getElementById('orig').value.trim();
  const alias = document.getElementById('alias').value.trim();
  const description = document.getElementById('createDesc').value.trim();
  const group_id = document.getElementById('createGroup').value;

  const payload = { original_url };
  if (alias) payload.alias = alias;
  if (description) payload.description = description;
  if (group_id) payload.group_id = group_id;

  const r = await api('/api/links', { method: 'POST', body: JSON.stringify(payload) });
  const out = document.getElementById('createOut');
  if (r.ok) {
    out.textContent = `Created ${r.body.short_url}`;
    document.getElementById('orig').value = '';
    document.getElementById('alias').value = '';
    document.getElementById('createDesc').value = '';
    document.getElementById('createGroup').value = '';
    await loadLinks();
  } else {
    out.textContent = `Error ${r.status}: ${(r.body?.error?.message) || 'failed'}`;
  }
}

function prevPage() {
  if (currentPage > 0) {
    currentPage--;
    loadLinks();
  }
}

function nextPage() {
  if (hasMore) {
    currentPage++;
    loadLinks();
  }
}

function onSearchInput() {
  clearTimeout(searchDebounce);
  searchDebounce = setTimeout(() => {
    currentPage = 0;
    loadLinks();
  }, 300);
}

// Event handlers
document.getElementById('createBtn').onclick = createLink;
document.getElementById('refresh').onclick = loadLinks;
document.getElementById('filterBy').onchange = () => { currentPage = 0; loadLinks(); };
document.getElementById('filterByGroup').onchange = () => { currentPage = 0; loadLinks(); };
document.getElementById('editCancel').onclick = closeEditModal;
document.getElementById('editSave').onclick = saveEdit;
document.getElementById('editDelete').onclick = () => deleteLink(document.getElementById('editModal').dataset.slug);
document.getElementById('qrClose').onclick = closeQrModal;
document.getElementById('qrMode').onchange = loadQrCode;
document.getElementById('searchInput').oninput = onSearchInput;
document.getElementById('prevPage').onclick = prevPage;
document.getElementById('nextPage').onclick = nextPage;
document.getElementById('prevPage2').onclick = prevPage;
document.getElementById('nextPage2').onclick = nextPage;
document.getElementById('bulkActivate').onclick = bulkActivate;
document.getElementById('bulkDeactivate').onclick = bulkDeactivate;
document.getElementById('bulkDelete').onclick = bulkDelete;

// Group management event handlers
document.getElementById('createGroupBtn').onclick = openGroupCreateModal;
document.getElementById('refreshGroups').onclick = loadGroups;
document.getElementById('groupCancel').onclick = closeGroupModal;
document.getElementById('groupSave').onclick = saveGroup;
document.getElementById('membersClose').onclick = closeMembersModal;
document.getElementById('addMemberBtn').onclick = addMember;

// Delegate checkbox change for selection count
document.querySelector('#listTbl tbody').addEventListener('change', (e) => {
  if (e.target.classList.contains('link-select')) {
    updateSelectedCount();
  }
});

window.addEventListener('load', initAuth);
