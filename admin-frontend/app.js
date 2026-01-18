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
  if (d < now) return `<span style="color:red;">Expired</span>`;
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
    if (hasNewFeatures) {
      const statusBadge = l.is_active
        ? '<span style="color:green;">Active</span>'
        : '<span style="color:red;">Inactive</span>';
      const expiresDisplay = l.expires_at
        ? `<span title="${l.expires_at}">${formatExpiry(l.expires_at)}</span>`
        : '-';
      const descTitle = l.description ? ` title="${l.description.replace(/"/g, '&quot;')}"` : '';
      const groupName = l.group_id ? (allGroups.find(g => g.id === l.group_id)?.name || l.group_id) : '-';
      tr.innerHTML = `
        <td><input type="checkbox" class="link-select" data-slug="${l.slug}" /></td>
        <td${descTitle}>${l.slug}${l.description ? ' *' : ''}</td>
        <td><a href="${l.short_url}" target="_blank" rel="noreferrer">${l.short_url}</a> <button class="copy-btn" onclick="copyToClipboard('${l.short_url}', this)" title="Copy">📋</button></td>
        <td style="max-width:300px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;" title="${l.original_url}">${l.original_url}</td>
        <td>${l.click_count}</td>
        <td>${statusBadge}</td>
        <td>${groupName}</td>
        <td>${expiresDisplay}</td>
        <td>${new Date(l.created_at).toLocaleDateString()}</td>
        <td>${l.created_by}</td>
        <td>
          <button onclick="openEditModal('${l.slug}')">Edit</button>
          <button onclick="toggleLink('${l.slug}', ${l.is_active})">${l.is_active ? 'Off' : 'On'}</button>
          <button onclick="showQrCode('${l.slug}', '${l.short_url}')">QR</button>
        </td>
      `;
    } else {
      tr.innerHTML = `
        <td><input type="checkbox" class="link-select" data-slug="${l.slug}" /></td>
        <td>${l.slug}</td>
        <td><a href="${l.short_url}" target="_blank" rel="noreferrer">${l.short_url}</a> <button class="copy-btn" onclick="copyToClipboard('${l.short_url}', this)" title="Copy">📋</button></td>
        <td>${l.original_url}</td>
        <td>${l.created_at}</td>
        <td>${l.created_by}</td>
      `;
    }
    out.appendChild(tr);
  }

  updateSelectedCount();
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
    const adminBadge = currentUser.is_admin ? ' <span style="background:#4CAF50;color:white;padding:2px 6px;border-radius:3px;font-size:0.8em;">Admin</span>' : '';
    userEl.innerHTML = `Logged in as: <strong>${currentUser.email}</strong>${adminBadge}`;
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
  document.getElementById('editGroup').value = link.group_id || '';

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

function loadQrCode() {
  const container = document.getElementById('qrContainer');
  const downloadLink = document.getElementById('qrDownload');
  const urlDisplay = document.getElementById('qrUrlDisplay');
  const mode = document.getElementById('qrMode').value;

  // Build the URL with the selected suffix
  const targetUrl = `${currentQrBaseUrl}${mode}`;
  const qrUrl = `${targetUrl}.qr`;

  container.innerHTML = '<p>Loading QR code...</p>';
  urlDisplay.textContent = `URL: ${targetUrl}`;

  fetch(qrUrl)
    .then(res => {
      if (!res.ok) throw new Error('Failed to load QR code');
      return res.text();
    })
    .then(svg => {
      container.innerHTML = svg;
      const svgEl = container.querySelector('svg');
      if (svgEl) {
        svgEl.style.width = '200px';
        svgEl.style.height = '200px';
      }
      const blob = new Blob([svg], { type: 'image/svg+xml' });
      downloadLink.href = URL.createObjectURL(blob);
      const suffix = mode ? `-${mode.replace('+', 'preview')}` : '';
      downloadLink.download = `${currentQrSlug}${suffix}-qr.svg`;
      downloadLink.style.display = 'inline-block';
    })
    .catch(err => {
      container.innerHTML = `<p style="color:red;">Error: ${err.message}</p>`;
      downloadLink.style.display = 'none';
    });
}

function closeQrModal() {
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

// Make functions available globally for onclick handlers
window.openEditModal = openEditModal;
window.toggleLink = toggleLink;
window.showQrCode = showQrCode;
window.copyToClipboard = copyToClipboard;

// ============================================================================
// Group Management
// ============================================================================

let allGroups = [];
let currentGroupId = null;

// Populate all group dropdowns with current groups
function populateGroupDropdowns() {
  const dropdowns = ['createGroup', 'editGroup', 'filterByGroup'];
  for (const id of dropdowns) {
    const el = document.getElementById(id);
    if (!el) continue;
    const currentValue = el.value;
    el.innerHTML = id === 'filterByGroup'
      ? '<option value="">All groups</option>'
      : '<option value="">No group</option>';
    for (const g of allGroups) {
      const opt = document.createElement('option');
      opt.value = g.id;
      opt.textContent = g.name;
      el.appendChild(opt);
    }
    el.value = currentValue;
  }
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
    const roleBadge = g.role
      ? `<span style="background:${g.role === 'admin' ? '#4CAF50' : g.role === 'editor' ? '#2196F3' : '#9e9e9e'};color:white;padding:2px 6px;border-radius:3px;font-size:0.8em;">${g.role}</span>`
      : '';
    const canManage = g.role === 'admin' || currentUser?.is_admin;
    tr.innerHTML = `
      <td><strong>${g.name}</strong></td>
      <td class="muted">${g.description || '-'}</td>
      <td>${roleBadge}</td>
      <td>${new Date(g.created_at).toLocaleDateString()}</td>
      <td>
        <button onclick="openMembersModal('${g.id}', '${g.name.replace(/'/g, "\\'")}')">Members</button>
        ${canManage ? `<button onclick="openGroupEditModal('${g.id}')">Edit</button>` : ''}
        ${canManage ? `<button onclick="deleteGroup('${g.id}')" style="color:#dc3545;">Delete</button>` : ''}
      </td>
    `;
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
    const roleBadge = `<span style="background:${m.role === 'admin' ? '#4CAF50' : m.role === 'editor' ? '#2196F3' : '#9e9e9e'};color:white;padding:2px 6px;border-radius:3px;font-size:0.8em;">${m.role}</span>`;
    tr.innerHTML = `
      <td>${m.email}</td>
      <td>${roleBadge}</td>
      <td>${new Date(m.added_at).toLocaleDateString()}</td>
      <td>
        ${canManage ? `<button onclick="removeMember('${encodeURIComponent(m.email)}')" style="color:#dc3545;">Remove</button>` : ''}
      </td>
    `;
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

// Make group functions globally available
window.openGroupEditModal = openGroupEditModal;
window.deleteGroup = deleteGroup;
window.openMembersModal = openMembersModal;
window.removeMember = removeMember;

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
