const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const test = require('node:test');
const vm = require('node:vm');

const app = readFileSync(require.resolve('../admin-frontend/app.js'), 'utf8');

class Element {
  children = [];
  style = {};
  dataset = {};
  value = '';
  textContent = '';
  events = {};

  constructor(tag) {
    this.tag = tag;
  }

  append(...children) { this.children.push(...children); }
  appendChild(child) { this.append(child); }
  replaceChildren(...children) {
    this.textContent = '';
    this.children = children;
  }
  addEventListener(name, listener) { this.events[name] = listener; }
  insertCell() {
    const cell = new Element('td');
    this.append(cell);
    return cell;
  }
  set innerHTML(value) {
    assert.doesNotMatch(value, /synthetic-marker/);
    this.children = [];
  }
}

function loadApp() {
  const elements = new Map();
  function element(key) {
    if (!elements.has(key)) elements.set(key, new Element(key));
    return elements.get(key);
  }
  const context = vm.createContext({
    URL,
    Option: class extends Element {
      constructor(text, value) {
        super('option');
        this.textContent = text;
        this.value = value;
      }
    },
    document: {
      createElement: tag => new Element(tag),
      getElementById: element,
      querySelector: element,
      querySelectorAll: () => [],
    },
    window: { location: { href: 'https://admin.example.test/' }, addEventListener() {} },
  });
  vm.runInContext(app, context);
  return { context, element, run: code => vm.runInContext(code, context) };
}

test('admin API values are rendered as literal text in links, groups and members', async () => {
  const { context, element, run } = loadApp();
  const marker = '<b title="synthetic-marker">Example</b>';
  const now = new Date().toISOString();
  context.responses = {
    '/api/links?limit=50&offset=0': {
      user: { email: marker, is_admin: true },
      links: [{
        slug: 'example',
        original_url: `https://example.test/${marker}`,
        short_url: 'https://short.example.test/example',
        description: marker,
        click_count: 0,
        group_id: 'team',
        created_by: marker,
        created_at: now,
        is_active: true,
      }],
    },
    '/api/groups': {
      groups: [{ id: 'team', name: marker, description: marker, role: 'admin', created_at: now }],
    },
    '/api/groups/team/members': { members: [{ email: marker, role: 'viewer', added_at: now }] },
  };
  await run(`
    api = async path => ({ ok: true, body: responses[path] });
    loadGroups().then(loadLinks).then(() => openMembersModal('team', responses['/api/groups'].groups[0].name))
  `);

  const [link] = element('#listTbl tbody').children;
  assert.equal(link.children[1].title, marker);
  assert.equal(link.children[2].children[0].href, 'https://short.example.test/example');
  assert.equal(link.children[3].textContent, `https://example.test/${marker}`);
  assert.equal(link.children[6].textContent, marker);
  assert.equal(link.children[9].textContent, marker);
  assert.equal(element('userInfo').children[1].textContent, marker);
  const [group] = element('#groupsTbl tbody').children;
  assert.equal(group.children[0].children[0].textContent, marker);
  assert.equal(group.children[1].textContent, marker);
  assert.equal(element('#membersTbl tbody').children[0].children[0].textContent, marker);
  await group.children[4].children[0].events.click();
  assert.equal(element('membersGroupName').textContent, marker);
  assert.equal(run('httpUrl("mailto:someone@example.test")'), null);
});

test('group selectors offer editor access and preserve the assigned viewer group', () => {
  const { element, run } = loadApp();
  run(`
    currentUser = { is_admin: false };
    allGroups = [
      { id: 'editor', name: 'Editor', role: 'editor' },
      { id: 'viewer', name: 'Viewer', role: 'viewer' },
      { id: 'other', name: 'Other', role: 'viewer' },
    ];
    allLinks = [{ slug: 'example', group_id: 'viewer' }];
    populateGroupDropdowns();
    openEditModal('example');
  `);
  const values = id => element(id).children.map(option => option.value);
  assert.deepEqual(values('createGroup'), ['', 'editor']);
  assert.deepEqual(values('editGroup'), ['', 'editor', 'viewer']);
  assert.equal(element('editGroup').value, 'viewer');
  assert.deepEqual(values('filterByGroup'), ['', 'editor', 'viewer', 'other']);
});
