// Task Tracker front-end: plain JavaScript, no framework and no build step.
//
// It talks to the JSON API defined in src/handlers.rs:
//
//   GET    /tasks          -> list all tasks (?filter=active|completed)
//   POST   /tasks          -> create a task
//   PATCH  /tasks/{id}     -> change title, description and/or completed
//   DELETE /tasks/{id}     -> delete a task
//
// Safety note: task text is only ever written with `textContent`, never with
// `innerHTML`, so a task titled "<script>..." is displayed as text instead of
// being executed by the browser.

const TASKS_URL = '/tasks';
const FILTERS = ['all', 'active', 'completed'];

const elements = {
  form: document.getElementById('create-form'),
  title: document.getElementById('create-title'),
  description: document.getElementById('create-description'),
  list: document.getElementById('task-list'),
  emptyState: document.getElementById('empty-state'),
  status: document.getElementById('status'),
  count: document.getElementById('task-count'),
  filterButtons: Array.from(document.querySelectorAll('.filter')),
};

/** Which filter is currently shown: 'all', 'active' or 'completed'. */
let currentFilter = 'all';

/**
 * Performs a request and returns the parsed JSON body (`null` for `204`).
 *
 * Non-2xx responses are turned into a thrown `Error` carrying the message the
 * server sent in `{"error": "..."}`, so the UI always shows the real reason.
 */
async function api(path, options = {}) {
  const response = await fetch(path, options);

  if (response.status === 204) {
    return null;
  }

  const text = await response.text();

  let payload = null;
  if (text) {
    try {
      payload = JSON.parse(text);
    } catch {
      payload = null; // e.g. a plain-text rejection from an extractor
    }
  }

  if (!response.ok) {
    const message =
      payload && typeof payload.error === 'string'
        ? payload.error
        : `Request failed with status ${response.status}`;
    throw new Error(message);
  }

  return payload;
}

/** Shows a message under the filters. */
function setStatus(message, isError = false) {
  elements.status.textContent = message;
  elements.status.classList.toggle('status--error', isError);
}

/** Shows a failed request in the status line. */
function showError(error) {
  setStatus(error instanceof Error ? error.message : 'Something went wrong.', true);
}

/** Fetches the tasks for the current filter and redraws the list. */
async function refresh() {
  const query = currentFilter === 'all' ? '' : `?filter=${currentFilter}`;

  try {
    renderTasks(await api(TASKS_URL + query));
  } catch (error) {
    showError(error);
  }
}

/** Replaces the list with the given tasks. */
function renderTasks(tasks) {
  elements.list.replaceChildren();

  elements.count.textContent =
    tasks.length === 1 ? '1 task' : `${tasks.length} tasks`;
  elements.emptyState.hidden = tasks.length > 0;

  for (const task of tasks) {
    elements.list.append(taskItem(task));
  }
}

/** Builds one `<li>` for a task. */
function taskItem(task) {
  const item = document.createElement('li');
  item.className = 'task';
  item.classList.toggle('task--completed', task.completed);

  const toggle = document.createElement('input');
  toggle.type = 'checkbox';
  toggle.className = 'task__toggle';
  toggle.checked = task.completed;
  toggle.setAttribute('aria-label', `Mark "${task.title}" as ${task.completed ? 'active' : 'completed'}`);
  toggle.addEventListener('change', () => {
    void updateTask(task.id, { completed: toggle.checked }, 'Task updated.');
  });

  const body = document.createElement('div');
  body.className = 'task__body';

  const title = document.createElement('p');
  title.className = 'task__title';
  title.textContent = task.title;
  body.append(title);

  if (task.description) {
    const description = document.createElement('p');
    description.className = 'task__description';
    description.textContent = task.description;
    body.append(description);
  }

  const actions = document.createElement('div');
  actions.className = 'task__actions';

  const edit = document.createElement('button');
  edit.type = 'button';
  edit.className = 'button';
  edit.textContent = 'Edit';
  edit.addEventListener('click', () => startEditing(item, task));

  const remove = document.createElement('button');
  remove.type = 'button';
  remove.className = 'button button--danger';
  remove.textContent = 'Delete';
  remove.addEventListener('click', () => void removeTask(task));

  actions.append(edit, remove);
  item.append(toggle, body, actions);

  return item;
}

/** Replaces a task row with a small edit form. */
function startEditing(item, task) {
  const form = document.createElement('form');
  form.className = 'task__edit';

  const titleInput = document.createElement('input');
  titleInput.type = 'text';
  titleInput.className = 'field__input';
  titleInput.maxLength = 200;
  titleInput.required = true;
  titleInput.value = task.title;
  titleInput.setAttribute('aria-label', 'Task title');

  const descriptionInput = document.createElement('textarea');
  descriptionInput.className = 'field__input';
  descriptionInput.rows = 2;
  descriptionInput.maxLength = 2000;
  descriptionInput.value = task.description ?? '';
  descriptionInput.placeholder = 'Description (leave empty for none)';
  descriptionInput.setAttribute('aria-label', 'Task description');

  const actions = document.createElement('div');
  actions.className = 'task__edit-actions';

  const save = document.createElement('button');
  save.type = 'submit';
  save.className = 'button button--primary';
  save.textContent = 'Save';

  const cancel = document.createElement('button');
  cancel.type = 'button';
  cancel.className = 'button';
  cancel.textContent = 'Cancel';
  // Nothing was sent yet, so cancelling simply redraws the stored task.
  cancel.addEventListener('click', () => void refresh());

  actions.append(save, cancel);
  form.append(titleInput, descriptionInput, actions);

  form.addEventListener('submit', (event) => {
    event.preventDefault();
    void updateTask(
      task.id,
      { title: titleInput.value, description: descriptionInput.value },
      'Task updated.',
    );
  });

  item.replaceChildren(form);
  titleInput.focus();
}

/** Sends a partial update and redraws the list on success. */
async function updateTask(id, changes, successMessage) {
  try {
    await api(`${TASKS_URL}/${id}`, {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(changes),
    });
    setStatus(successMessage);
    await refresh();
  } catch (error) {
    // The edit form stays on screen so the user can fix the input.
    showError(error);
  }
}

/** Creates a task from the form. */
async function createTask(event) {
  event.preventDefault();

  const title = elements.title.value.trim();
  const description = elements.description.value.trim();

  if (!title) {
    setStatus('Please enter a title.', true);
    elements.title.focus();
    return;
  }

  try {
    await api(TASKS_URL, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ title, description }),
    });

    elements.form.reset();
    elements.title.focus();
    setStatus('Task created.');
    await refresh();
  } catch (error) {
    showError(error);
  }
}

/** Deletes a task after a confirmation. */
async function removeTask(task) {
  if (!window.confirm(`Delete "${task.title}"?`)) {
    return;
  }

  try {
    await api(`${TASKS_URL}/${task.id}`, { method: 'DELETE' });
    setStatus('Task deleted.');
    await refresh();
  } catch (error) {
    showError(error);
  }
}

/** Highlights the chosen filter button. */
function setFilter(filter) {
  currentFilter = filter;

  for (const button of elements.filterButtons) {
    const isActive = button.dataset.filter === filter;
    button.classList.toggle('is-active', isActive);
    button.setAttribute('aria-pressed', String(isActive));
  }
}

elements.form.addEventListener('submit', (event) => void createTask(event));

for (const button of elements.filterButtons) {
  button.addEventListener('click', () => {
    const filter = button.dataset.filter;
    if (!FILTERS.includes(filter)) {
      return;
    }
    setFilter(filter);
    void refresh();
  });
}

void refresh();
