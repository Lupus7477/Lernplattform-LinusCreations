// === API Client ===
const API_BASE = '/api/v1';

async function api(endpoint, options = {}) {
    const url = `${API_BASE}${endpoint}`;
    const config = {
        headers: {
            'Content-Type': 'application/json',
            ...options.headers
        },
        ...options
    };

    try {
        const response = await fetch(url, config);
        const data = await response.json();
        
        if (!response.ok) {
            throw new Error(data.error || 'API-Fehler');
        }
        
        return data;
    } catch (error) {
        console.error('API Error:', error);
        throw error;
    }
}

// === State Management ===
const state = {
    documents: [],
    activePlan: null,
    topics: [],
    currentTopic: null,
    currentQuestions: [],
    currentQuestionIndex: 0,
    quizResults: { correct: 0, total: 0 },
    chatSessionId: `chat_${Date.now()}`,
    // Gamification
    wrongQuestions: [], // Falsch beantwortete Fragen
    points: parseInt(localStorage.getItem('quiz_points') || '0'),
    streak: parseInt(localStorage.getItem('quiz_streak') || '0'),
    totalCorrect: parseInt(localStorage.getItem('quiz_total_correct') || '0'),
    totalAnswered: parseInt(localStorage.getItem('quiz_total_answered') || '0')
};

// === Settings ===
function getSettings() {
    return {
        questionsCount: parseInt(localStorage.getItem('questions_count') || '5'),
        repeatWrongQuestions: localStorage.getItem('repeat_wrong') !== 'false',
        gamificationEnabled: localStorage.getItem('gamification') !== 'false'
    };
}

function saveSettings() {
    localStorage.setItem('questions_count', document.getElementById('questions-count').value);
    localStorage.setItem('repeat_wrong', document.getElementById('repeat-wrong-questions').checked);
    localStorage.setItem('gamification', document.getElementById('gamification-enabled').checked);
}

function loadSettingsUI() {
    const settings = getSettings();
    document.getElementById('questions-count').value = settings.questionsCount;
    document.getElementById('repeat-wrong-questions').checked = settings.repeatWrongQuestions;
    document.getElementById('gamification-enabled').checked = settings.gamificationEnabled;
    
    // Event Listeners für Auto-Save
    document.getElementById('questions-count').addEventListener('change', saveSettings);
    document.getElementById('repeat-wrong-questions').addEventListener('change', saveSettings);
    document.getElementById('gamification-enabled').addEventListener('change', saveSettings);
}

// === Navigation ===
function initNavigation() {
    document.querySelectorAll('.nav-item').forEach(item => {
        item.addEventListener('click', () => {
            const view = item.dataset.view;
            switchView(view);
            
            // Update active state
            document.querySelectorAll('.nav-item').forEach(i => i.classList.remove('active'));
            item.classList.add('active');
            
            // Schließe Mobile-Menü nach Auswahl
            closeMobileMenu();
        });
    });
}

// === Mobile Menu ===
function toggleMobileMenu() {
    const sidebar = document.getElementById('sidebar');
    const overlay = document.querySelector('.sidebar-overlay');
    sidebar.classList.toggle('open');
    overlay.classList.toggle('active');
}

function closeMobileMenu() {
    const sidebar = document.getElementById('sidebar');
    const overlay = document.querySelector('.sidebar-overlay');
    sidebar.classList.remove('open');
    overlay.classList.remove('active');
}

function switchView(viewName) {
    document.querySelectorAll('.view').forEach(v => v.classList.remove('active'));
    document.getElementById(`view-${viewName}`).classList.add('active');
    
    // Load view-specific data
    switch(viewName) {
        case 'dashboard':
            loadDashboard();
            break;
        case 'documents':
            loadDocuments();
            break;
        case 'study-plan':
            loadStudyPlan();
            break;
        case 'learn':
            loadLearnView();
            break;
        case 'quiz':
            loadQuizView();
            break;
        case 'chat':
            loadChatTopics();
            break;
        case 'glossary':
            loadGlossary();
            break;
        case 'settings':
            loadSettings();
            break;
    }
}

// === Dashboard ===
async function loadDashboard() {
    try {
        const status = await api('/status');
        
        // Update stats
        document.getElementById('stat-documents').textContent = status.documents_count || 0;
        
        // Update LLM status
        const llmStatus = document.getElementById('llm-status');
        if (status.llm_available) {
            llmStatus.textContent = `🤖 ${status.llm_provider} Online`;
            llmStatus.className = 'status-badge status-online';
        } else {
            llmStatus.textContent = '🤖 LLM Offline';
            llmStatus.className = 'status-badge status-offline';
        }

        // Check steps
        updateSteps(status);

        // Load progress if plan exists
        if (status.active_plan) {
            await loadProgress();
        }
    } catch (error) {
        console.error('Dashboard laden fehlgeschlagen:', error);
    }
}

function updateSteps(status) {
    const step1 = document.getElementById('step-1');
    const step2 = document.getElementById('step-2');
    const step3 = document.getElementById('step-3');
    const step4 = document.getElementById('step-4');

    if (status.documents_count > 0) {
        step1.classList.add('completed');
    }
    if (status.active_plan) {
        step2.classList.add('completed');
        step3.classList.add('completed');
    }
}

async function loadProgress() {
    try {
        const progress = await api('/progress');
        
        document.getElementById('stat-days').textContent = progress.days_until_exam;
        document.getElementById('stat-progress').textContent = 
            `${Math.round((progress.completed_topics / progress.total_topics) * 100)}%`;
        document.getElementById('stat-score').textContent = 
            progress.average_score ? `${Math.round(progress.average_score)}%` : '-';

        // Update progress overview
        const overview = document.getElementById('progress-overview');
        overview.innerHTML = `
            <div class="progress-bar">
                <div class="progress-fill" style="width: ${(progress.completed_topics / progress.total_topics) * 100}%"></div>
            </div>
            <p>${progress.completed_topics} von ${progress.total_topics} Themen abgeschlossen</p>
            <p>${progress.answered_questions} Fragen beantwortet, ${progress.correct_answers} richtig</p>
        `;
    } catch (error) {
        console.log('Kein aktiver Lernplan');
    }
}

// === Documents ===
async function loadDocuments() {
    try {
        const data = await api('/documents');
        state.documents = data.documents || [];
        renderDocuments();
    } catch (error) {
        console.error('Dokumente laden fehlgeschlagen:', error);
    }
}

function renderDocuments() {
    const container = document.getElementById('documents-list');
    
    if (state.documents.length === 0) {
        container.innerHTML = '<p class="placeholder">Noch keine Dokumente hochgeladen.</p>';
        return;
    }

    container.innerHTML = state.documents.map(doc => `
        <div class="document-item" data-id="${doc.id}">
            <div class="document-info">
                <span class="document-icon">📄</span>
                <div>
                    <div class="document-name">${doc.name}</div>
                    <div class="document-meta">${doc.page_count} Seiten</div>
                </div>
            </div>
            <button class="btn btn-secondary" onclick="deleteDocument('${doc.id}')">🗑️</button>
        </div>
    `).join('');
}

function initDocumentUpload() {
    const uploadArea = document.getElementById('upload-area');
    const fileInput = document.getElementById('file-input');

    uploadArea.addEventListener('click', () => fileInput.click());
    
    uploadArea.addEventListener('dragover', (e) => {
        e.preventDefault();
        uploadArea.classList.add('dragover');
    });

    uploadArea.addEventListener('dragleave', () => {
        uploadArea.classList.remove('dragover');
    });

    uploadArea.addEventListener('drop', async (e) => {
        e.preventDefault();
        uploadArea.classList.remove('dragover');
        const files = e.dataTransfer.files;
        await uploadFiles(files);
    });

    fileInput.addEventListener('change', async (e) => {
        await uploadFiles(e.target.files);
    });

    document.getElementById('scan-folder-btn').addEventListener('click', scanFolder);
}

async function uploadFiles(files) {
    for (const file of files) {
        if (file.type !== 'application/pdf') {
            alert('Nur PDF-Dateien werden unterstützt.');
            continue;
        }

        const formData = new FormData();
        formData.append('file', file);

        try {
            await fetch(`${API_BASE}/documents`, {
                method: 'POST',
                body: formData
            });
        } catch (error) {
            console.error('Upload fehlgeschlagen:', error);
        }
    }
    
    await loadDocuments();
}

async function scanFolder() {
    try {
        const data = await api('/documents/scan', { method: 'POST' });
        alert(data.message);
        await loadDocuments();
    } catch (error) {
        alert('Fehler beim Scannen: ' + error.message);
    }
}

async function deleteDocument(id) {
    if (!confirm('Dokument wirklich löschen?')) return;
    
    try {
        await api(`/documents/${id}`, { method: 'DELETE' });
        await loadDocuments();
    } catch (error) {
        alert('Fehler beim Löschen: ' + error.message);
    }
}

// === Study Plan ===
async function loadStudyPlan() {
    await loadDocuments();
    
    // Populate document selection
    const container = document.getElementById('plan-documents');
    if (state.documents.length === 0) {
        container.innerHTML = '<p class="placeholder">Lade zuerst Dokumente hoch.</p>';
    } else {
        container.innerHTML = state.documents.map(doc => `
            <label class="checkbox-item">
                <input type="checkbox" name="doc" value="${doc.id}" checked>
                <span>${doc.name}</span>
            </label>
        `).join('');
    }

    // Try to load active plan
    try {
        const plan = await api('/plans/active');
        state.activePlan = plan;
        renderActivePlan(plan);
    } catch (error) {
        document.getElementById('no-plan').classList.remove('hidden');
        document.getElementById('active-plan').classList.add('hidden');
    }
}

function initStudyPlanForm() {
    document.getElementById('create-plan-form').addEventListener('submit', async (e) => {
        e.preventDefault();
        
        const examDate = document.getElementById('exam-date').value;
        const selectedDocs = Array.from(document.querySelectorAll('input[name="doc"]:checked'))
            .map(cb => cb.value);

        if (selectedDocs.length === 0) {
            alert('Bitte wähle mindestens ein Dokument aus.');
            return;
        }

        const btn = document.getElementById('create-plan-btn');
        btn.disabled = true;
        btn.textContent = '⏳ Erstelle Lernplan...';

        try {
            const plan = await api('/plans', {
                method: 'POST',
                body: JSON.stringify({
                    exam_date: examDate,
                    document_ids: selectedDocs
                })
            });
            
            state.activePlan = plan;
            renderActivePlan(plan);
        } catch (error) {
            alert('Fehler beim Erstellen: ' + error.message);
        } finally {
            btn.disabled = false;
            btn.textContent = '🚀 Lernplan erstellen';
        }
    });
}

function renderActivePlan(plan) {
    document.getElementById('no-plan').classList.add('hidden');
    document.getElementById('active-plan').classList.remove('hidden');

    document.getElementById('plan-name').textContent = plan.name;
    document.getElementById('plan-exam-date').textContent = 
        new Date(plan.exam_date).toLocaleDateString('de-DE');
    
    const daysLeft = Math.ceil((new Date(plan.exam_date) - new Date()) / (1000 * 60 * 60 * 24));
    document.getElementById('plan-days-left').textContent = Math.max(0, daysLeft);
    
    document.getElementById('plan-progress').textContent = Math.round(plan.progress);
    document.getElementById('plan-progress-bar').style.width = `${plan.progress}%`;

    // Render topics
    const topicsContainer = document.getElementById('topics-list');
    state.topics = plan.topics || [];
    
    topicsContainer.innerHTML = state.topics.map((topic, index) => `
        <div class="topic-item" onclick="openTopic('${topic.id}')">
            <div class="topic-info">
                <div class="topic-name">${index + 1}. ${topic.name}</div>
                <div class="topic-description">${topic.description}</div>
            </div>
            <span class="topic-status ${topic.status}">${getStatusLabel(topic.status)}</span>
        </div>
    `).join('');
}

function getStatusLabel(status) {
    const labels = {
        'pending': 'Offen',
        'in_progress': 'In Arbeit',
        'completed': 'Abgeschlossen'
    };
    return labels[status] || status;
}

function openTopic(topicId) {
    state.currentTopic = state.topics.find(t => t.id === topicId);
    switchView('learn');
    document.querySelector('[data-view="learn"]').click();
}

// === Learn View ===
async function loadLearnView() {
    if (!state.activePlan) {
        try {
            const plan = await api('/plans/active');
            state.activePlan = plan;
            state.topics = plan.topics || [];
        } catch {
            document.getElementById('learn-select-topic').innerHTML = 
                '<p class="placeholder">Erstelle zuerst einen Lernplan.</p>';
            return;
        }
    }

    if (state.currentTopic) {
        showTopicContent();
    } else {
        showTopicSelection();
    }
}

function showTopicSelection() {
    document.getElementById('learn-select-topic').classList.remove('hidden');
    document.getElementById('learn-content').classList.add('hidden');

    const container = document.getElementById('learn-topics');
    container.innerHTML = state.topics.map(topic => `
        <div class="topic-card" onclick="selectLearnTopic('${topic.id}')">
            <div class="topic-name">${topic.name}</div>
            <div class="topic-status ${topic.status}">${getStatusLabel(topic.status)}</div>
        </div>
    `).join('');
}

async function selectLearnTopic(topicId) {
    state.currentTopic = state.topics.find(t => t.id === topicId);
    await showTopicContent();
}

async function showTopicContent() {
    document.getElementById('learn-select-topic').classList.add('hidden');
    document.getElementById('learn-content').classList.remove('hidden');

    document.getElementById('learn-topic-title').textContent = state.currentTopic.name;
    
    const container = document.getElementById('explanation-content');
    // Zeige Skeleton Loading
    container.innerHTML = `
        <div class="skeleton-container">
            <div class="skeleton skeleton-title"></div>
            <div class="skeleton skeleton-text"></div>
            <div class="skeleton skeleton-text"></div>
            <div class="skeleton skeleton-text short"></div>
            <div class="skeleton skeleton-title" style="margin-top: 24px;"></div>
            <div class="skeleton skeleton-text"></div>
            <div class="skeleton skeleton-text"></div>
            <div class="skeleton skeleton-text short"></div>
        </div>
    `;

    try {
        const explanation = await api(`/topics/${state.currentTopic.id}/explain`);
        container.innerHTML = `<div class="markdown-body">${formatExplanation(explanation.content)}</div>`;
        
        // Highlight code blocks
        if (typeof hljs !== 'undefined') {
            container.querySelectorAll('pre code').forEach((block) => {
                hljs.highlightElement(block);
            });
        }
        
        // Mache alle Fachbegriffe (strong) klickbar
        initInteractiveTerms(container);
    } catch (error) {
        container.innerHTML = `<p class="placeholder">Fehler: ${error.message}</p>`;
    }
}

// === Interaktive Begriffe ===
let activeTooltip = null;

function initInteractiveTerms(container) {
    const terms = container.querySelectorAll('.markdown-body strong');
    
    terms.forEach(term => {
        term.addEventListener('click', (e) => {
            e.stopPropagation();
            showTermTooltip(term);
        });
    });
    
    // Schließe Tooltip wenn außerhalb geklickt
    document.addEventListener('click', closeTermTooltip);
}

function showTermTooltip(termElement) {
    closeTermTooltip();
    
    const term = termElement.textContent.trim();
    const rect = termElement.getBoundingClientRect();
    
    const tooltip = document.createElement('div');
    tooltip.className = 'term-tooltip';
    tooltip.innerHTML = `
        <div class="term-tooltip-header">
            <span class="term-tooltip-title">${term}</span>
            <button class="term-tooltip-close" onclick="closeTermTooltip()">×</button>
        </div>
        <div class="term-tooltip-content">
            <div class="skeleton skeleton-text" style="width: 100%; height: 14px; margin-bottom: 8px;"></div>
            <div class="skeleton skeleton-text" style="width: 80%; height: 14px;"></div>
        </div>
        <div class="term-tooltip-actions">
            <button class="term-tooltip-btn secondary" onclick="closeTermTooltip()">Schließen</button>
            <button class="term-tooltip-btn primary" onclick="addTermToGlossary('${term.replace(/'/g, "\\'")}')">📖 Zum Glossar</button>
        </div>
    `;
    
    // Positioniere Tooltip
    document.body.appendChild(tooltip);
    
    const tooltipRect = tooltip.getBoundingClientRect();
    let top = rect.bottom + 8;
    let left = rect.left;
    
    // Prüfe ob Tooltip aus dem Viewport ragt
    if (left + tooltipRect.width > window.innerWidth - 16) {
        left = window.innerWidth - tooltipRect.width - 16;
    }
    if (top + tooltipRect.height > window.innerHeight - 16) {
        top = rect.top - tooltipRect.height - 8;
    }
    
    tooltip.style.top = `${top}px`;
    tooltip.style.left = `${left}px`;
    
    activeTooltip = tooltip;
    
    // Lade KI-Erklärung für den Begriff
    loadTermExplanation(term, tooltip);
}

async function loadTermExplanation(term, tooltip) {
    const contentDiv = tooltip.querySelector('.term-tooltip-content');
    
    try {
        // Schnelle KI-Anfrage für Begriffserklärung
        const response = await api('/chat', {
            method: 'POST',
            body: JSON.stringify({
                message: `Erkläre den Begriff "${term}" in maximal 2 kurzen Sätzen. Nur die Definition, keine Einleitung.`,
                session_id: 'term_explain'
            })
        });
        
        contentDiv.innerHTML = `<p>${response.response}</p>`;
        
        // Prüfe ob Begriff schon im Glossar ist
        checkIfTermInGlossary(term, tooltip);
    } catch (error) {
        contentDiv.innerHTML = `<p style="color: #6b7280;">Klicke auf "Zum Glossar" um den Begriff zu speichern.</p>`;
    }
}

async function checkIfTermInGlossary(term, tooltip) {
    try {
        const glossary = await api('/glossary');
        const exists = glossary.some(item => 
            item.term.toLowerCase() === term.toLowerCase()
        );
        
        if (exists) {
            const header = tooltip.querySelector('.term-tooltip-header');
            header.innerHTML += '<span class="term-in-glossary">✓ Im Glossar</span>';
            
            const addBtn = tooltip.querySelector('.term-tooltip-btn.primary');
            addBtn.textContent = '✓ Bereits gespeichert';
            addBtn.classList.remove('primary');
            addBtn.classList.add('success');
            addBtn.disabled = true;
        }
    } catch (e) {
        // Glossar nicht verfügbar
    }
}

async function addTermToGlossary(term) {
    const tooltip = activeTooltip;
    const contentDiv = tooltip?.querySelector('.term-tooltip-content p');
    const definition = contentDiv?.textContent || '';
    
    try {
        await api('/glossary', {
            method: 'POST',
            body: JSON.stringify({
                term: term,
                category: state.currentTopic?.name || 'Allgemein',
                definition: definition,
                details: `Aus dem Thema: ${state.currentTopic?.name || 'Lernen'}`,
                related: []
            })
        });
        
        // Update Button
        const addBtn = tooltip.querySelector('.term-tooltip-btn.primary');
        addBtn.textContent = '✓ Gespeichert!';
        addBtn.classList.remove('primary');
        addBtn.classList.add('success');
        addBtn.disabled = true;
        
        // Füge Badge hinzu
        const header = tooltip.querySelector('.term-tooltip-header');
        if (!header.querySelector('.term-in-glossary')) {
            header.innerHTML += '<span class="term-in-glossary">✓ Im Glossar</span>';
        }
    } catch (error) {
        alert('Fehler beim Speichern: ' + error.message);
    }
}

function closeTermTooltip() {
    if (activeTooltip) {
        activeTooltip.remove();
        activeTooltip = null;
    }
}

function formatExplanation(content) {
    // Markdown mit marked.js rendern
    if (typeof marked !== 'undefined') {
        marked.setOptions({
            highlight: function(code, lang) {
                if (typeof hljs !== 'undefined' && lang && hljs.getLanguage(lang)) {
                    return hljs.highlight(code, { language: lang }).value;
                }
                return code;
            },
            breaks: true,
            gfm: true
        });
        return marked.parse(content);
    }
    
    // Fallback: Simple markdown-like formatting
    return content
        .replace(/\n\n/g, '</p><p>')
        .replace(/\*\*(.*?)\*\*/g, '<strong>$1</strong>')
        .replace(/\*(.*?)\*/g, '<em>$1</em>')
        .replace(/`([^`]+)`/g, '<code>$1</code>')
        .replace(/\n- /g, '<br>• ')
        .replace(/^/, '<p>')
        .replace(/$/, '</p>');
}

function initLearnButtons() {
    document.getElementById('back-to-topics').addEventListener('click', () => {
        state.currentTopic = null;
        showTopicSelection();
    });

    document.getElementById('start-quiz-btn').addEventListener('click', () => {
        switchView('quiz');
        document.querySelector('[data-view="quiz"]').click();
    });

    document.getElementById('mark-complete-btn').addEventListener('click', async () => {
        try {
            await api(`/topics/${state.currentTopic.id}/status`, {
                method: 'PUT',
                body: JSON.stringify({ status: 'completed', progress: 100 })
            });
            state.currentTopic.status = 'completed';
            alert('Thema als gelernt markiert!');
        } catch (error) {
            alert('Fehler: ' + error.message);
        }
    });
}

// === Quiz View ===
async function loadQuizView() {
    if (!state.activePlan) {
        try {
            const plan = await api('/plans/active');
            state.activePlan = plan;
            state.topics = plan.topics || [];
        } catch {
            return;
        }
    }

    showQuizSelect();
}

function showQuizSelect() {
    document.getElementById('quiz-select').classList.remove('hidden');
    document.getElementById('quiz-active').classList.add('hidden');
    document.getElementById('quiz-results').classList.add('hidden');

    const container = document.getElementById('quiz-topics');
    container.innerHTML = state.topics.map(topic => `
        <div class="topic-card" data-id="${topic.id}" onclick="selectQuizTopic(this, '${topic.id}')">
            <div class="topic-name">${topic.name}</div>
        </div>
    `).join('');
}

let selectedQuizTopicId = null;

function selectQuizTopic(element, topicId) {
    document.querySelectorAll('#quiz-topics .topic-card').forEach(el => {
        el.classList.remove('selected');
    });
    element.classList.add('selected');
    selectedQuizTopicId = topicId;
    
    // Start quiz automatically
    setTimeout(startQuiz, 300);
}

async function startQuiz() {
    if (!selectedQuizTopicId) {
        alert('Bitte wähle ein Thema aus.');
        return;
    }

    const difficulty = parseInt(document.getElementById('difficulty-select').value);
    const settings = getSettings();
    
    document.getElementById('quiz-select').classList.add('hidden');
    document.getElementById('quiz-active').classList.remove('hidden');
    
    // Reset für neues Quiz
    state.wrongQuestions = [];
    
    // Zeige Skeleton während Fragen laden
    document.getElementById('question-text').innerHTML = `
        <div class="skeleton skeleton-text" style="width: 100%; height: 20px; margin-bottom: 12px;"></div>
        <div class="skeleton skeleton-text" style="width: 90%; height: 20px; margin-bottom: 12px;"></div>
        <div class="skeleton skeleton-text" style="width: 70%; height: 20px;"></div>
    `;
    document.querySelector('.answer-section').classList.add('hidden');

    try {
        // Prüfe zuerst ob gecachte Fragen existieren
        let questions;
        try {
            const cached = await api(`/topics/${selectedQuizTopicId}/questions?difficulty=${difficulty}`);
            if (cached && cached.length >= settings.questionsCount) {
                questions = cached.slice(0, settings.questionsCount);
            }
        } catch (e) {
            // Keine gecachten Fragen
        }
        
        // Falls keine gecachten, generiere neue
        if (!questions || questions.length < settings.questionsCount) {
            questions = await api(`/topics/${selectedQuizTopicId}/questions/generate`, {
                method: 'POST',
                body: JSON.stringify({ difficulty, count: settings.questionsCount })
            });
        }

        state.currentQuestions = questions;
        state.currentQuestionIndex = 0;
        state.quizResults = { correct: 0, total: questions.length };

        showQuestion();
    } catch (error) {
        alert('Fehler beim Laden der Fragen: ' + error.message);
        showQuizSelect();
    }
}

function showQuestion() {
    const question = state.currentQuestions[state.currentQuestionIndex];
    
    document.getElementById('quiz-current').textContent = state.currentQuestionIndex + 1;
    document.getElementById('quiz-total').textContent = state.currentQuestions.length;
    document.getElementById('quiz-difficulty').textContent = '⭐'.repeat(question.difficulty);
    
    document.getElementById('question-text').textContent = question.question;
    document.getElementById('answer-input').value = '';
    
    // Hide feedback, show answer section
    document.getElementById('feedback-section').classList.add('hidden');
    document.querySelector('.answer-section').classList.remove('hidden');

    // Setup hints
    const hintsContainer = document.getElementById('question-hints');
    const hintText = document.getElementById('hint-text');
    if (question.hints && question.hints.length > 0) {
        hintsContainer.classList.remove('hidden');
        hintText.textContent = question.hints[0];
        hintText.classList.add('hidden');
    } else {
        hintsContainer.classList.add('hidden');
    }
}

function initQuizButtons() {
    document.getElementById('show-hint-btn').addEventListener('click', () => {
        document.getElementById('hint-text').classList.toggle('hidden');
    });

    document.getElementById('submit-answer-btn').addEventListener('click', submitAnswer);
    
    document.getElementById('next-question-btn').addEventListener('click', () => {
        state.currentQuestionIndex++;
        if (state.currentQuestionIndex >= state.currentQuestions.length) {
            showQuizResults();
        } else {
            showQuestion();
        }
    });

    document.getElementById('retry-quiz-btn').addEventListener('click', startQuiz);
    document.getElementById('new-quiz-btn').addEventListener('click', showQuizSelect);
}

async function submitAnswer() {
    const answer = document.getElementById('answer-input').value.trim();
    if (!answer) {
        alert('Bitte gib eine Antwort ein.');
        return;
    }

    const question = state.currentQuestions[state.currentQuestionIndex];
    const settings = getSettings();
    
    try {
        const result = await api(`/questions/${question.id}/answer`, {
            method: 'POST',
            body: JSON.stringify({ answer })
        });

        // Gamification
        state.totalAnswered++;
        localStorage.setItem('quiz_total_answered', state.totalAnswered);
        
        if (result.is_correct) {
            state.quizResults.correct++;
            state.totalCorrect++;
            state.streak++;
            
            // Punkte basierend auf Streak
            const pointsEarned = 10 + (state.streak * 2);
            state.points += pointsEarned;
            
            localStorage.setItem('quiz_total_correct', state.totalCorrect);
            localStorage.setItem('quiz_streak', state.streak);
            localStorage.setItem('quiz_points', state.points);
            
            result.pointsEarned = pointsEarned;
            result.streak = state.streak;
        } else {
            state.streak = 0;
            localStorage.setItem('quiz_streak', 0);
            
            // Merke falsche Frage für Wiederholung
            if (settings.repeatWrongQuestions && !question.isRetry) {
                state.wrongQuestions.push({...question, isRetry: true});
            }
        }

        showFeedback(result);
    } catch (error) {
        alert('Fehler bei der Bewertung: ' + error.message);
    }
}

function showFeedback(result) {
    document.querySelector('.answer-section').classList.add('hidden');
    
    const feedback = document.getElementById('feedback-section');
    feedback.classList.remove('hidden', 'correct', 'incorrect');
    feedback.classList.add(result.is_correct ? 'correct' : 'incorrect');
    
    const settings = getSettings();
    
    if (result.is_correct) {
        let gamificationText = '';
        if (settings.gamificationEnabled && result.pointsEarned) {
            gamificationText = `<div class="gamification-bonus">
                <span class="points-earned">+${result.pointsEarned} Punkte! 🎉</span>
                ${result.streak > 1 ? `<span class="streak-bonus">🔥 ${result.streak}er Streak!</span>` : ''}
            </div>`;
        }
        document.getElementById('feedback-icon').textContent = '✅';
        document.getElementById('feedback-status').innerHTML = 'Richtig!' + gamificationText;
    } else {
        document.getElementById('feedback-icon').textContent = '❌';
        document.getElementById('feedback-status').textContent = 'Noch nicht ganz...';
    }
    
    document.getElementById('feedback-text').textContent = result.feedback;
    
    document.getElementById('expected-answer').innerHTML = `
        <strong>💡 Die richtige Antwort:</strong><br>${result.expected}
    `;
}

function showQuizResults() {
    const settings = getSettings();
    
    // Prüfe ob noch falsche Fragen wiederholt werden sollen
    if (settings.repeatWrongQuestions && state.wrongQuestions.length > 0) {
        // Füge falsche Fragen ans Ende hinzu
        state.currentQuestions = [...state.currentQuestions, ...state.wrongQuestions];
        state.wrongQuestions = [];
        state.quizResults.total = state.currentQuestions.length;
        
        // Zeige Motivations-Nachricht
        showRetryMessage();
        return;
    }
    
    document.getElementById('quiz-active').classList.add('hidden');
    document.getElementById('quiz-results').classList.remove('hidden');

    const percent = Math.round((state.quizResults.correct / state.quizResults.total) * 100);
    
    document.getElementById('result-correct').textContent = state.quizResults.correct;
    document.getElementById('result-total').textContent = state.quizResults.total;
    document.getElementById('result-percent').textContent = `${percent}%`;
    
    // Gamification Ergebnis
    if (settings.gamificationEnabled) {
        showGamificationResults(percent);
    }
}

function showRetryMessage() {
    const retryCount = state.currentQuestions.length - state.currentQuestionIndex;
    
    // Kurze Motivations-Nachricht anzeigen
    const questionText = document.getElementById('question-text');
    questionText.innerHTML = `
        <div class="retry-message">
            <div class="retry-icon">🎮</div>
            <h3>Bonusrunde!</h3>
            <p>Du hast ${retryCount} Frage${retryCount > 1 ? 'n' : ''} noch nicht ganz richtig.<br>
            Lass uns die nochmal üben!</p>
            <p class="retry-tip">💡 Tipp: Diesmal schaffst du es!</p>
        </div>
    `;
    
    document.querySelector('.answer-section').classList.add('hidden');
    document.getElementById('feedback-section').classList.add('hidden');
    
    // Nach 2 Sekunden automatisch weitermachen
    setTimeout(() => {
        showQuestion();
    }, 2500);
}

function showGamificationResults(percent) {
    const resultsCard = document.getElementById('quiz-results');
    
    // Berechne Achievement
    let achievement = '';
    if (percent === 100) {
        achievement = '🏆 Perfekt! Du bist ein Experte!';
    } else if (percent >= 80) {
        achievement = '⭐ Großartig! Fast perfekt!';
    } else if (percent >= 60) {
        achievement = '👍 Gut gemacht! Weiter so!';
    } else {
        achievement = '💪 Übung macht den Meister!';
    }
    
    // Füge Gamification-Stats hinzu
    const existingStats = resultsCard.querySelector('.gamification-stats');
    if (existingStats) existingStats.remove();
    
    const statsDiv = document.createElement('div');
    statsDiv.className = 'gamification-stats';
    statsDiv.innerHTML = `
        <div class="achievement-banner">${achievement}</div>
        <div class="stats-row">
            <div class="stat-item">
                <span class="stat-icon">🎯</span>
                <span class="stat-value">${state.points}</span>
                <span class="stat-label">Punkte gesamt</span>
            </div>
            <div class="stat-item">
                <span class="stat-icon">📊</span>
                <span class="stat-value">${Math.round((state.totalCorrect / Math.max(state.totalAnswered, 1)) * 100)}%</span>
                <span class="stat-label">Gesamtquote</span>
            </div>
            <div class="stat-item">
                <span class="stat-icon">🔥</span>
                <span class="stat-value">${state.streak}</span>
                <span class="stat-label">Aktueller Streak</span>
            </div>
        </div>
    `;
    
    resultsCard.querySelector('.card').appendChild(statsDiv);
}

// === Chat ===
async function loadChatTopics() {
    if (state.topics.length > 0) {
        const select = document.getElementById('chat-topic-select');
        select.innerHTML = '<option value="">Allgemein</option>' +
            state.topics.map(t => `<option value="${t.id}">${t.name}</option>`).join('');
    }
}

function initChat() {
    document.getElementById('send-chat-btn').addEventListener('click', sendChatMessage);
    document.getElementById('chat-input').addEventListener('keypress', (e) => {
        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            sendChatMessage();
        }
    });
}

async function sendChatMessage() {
    const input = document.getElementById('chat-input');
    const message = input.value.trim();
    if (!message) return;

    const topicId = document.getElementById('chat-topic-select').value;
    
    // Add user message to UI
    addChatMessage(message, 'user');
    input.value = '';

    try {
        const response = await api('/chat', {
            method: 'POST',
            body: JSON.stringify({
                message,
                topic_id: topicId,
                session_id: state.chatSessionId
            })
        });

        addChatMessage(response.response, 'assistant');
    } catch (error) {
        addChatMessage('Entschuldigung, es gab einen Fehler: ' + error.message, 'assistant');
    }
}

function addChatMessage(content, role) {
    const container = document.getElementById('chat-messages');
    const messageDiv = document.createElement('div');
    messageDiv.className = `chat-message ${role}`;
    messageDiv.innerHTML = `<div class="message-content">${content}</div>`;
    container.appendChild(messageDiv);
    container.scrollTop = container.scrollHeight;
}

// === Settings ===
async function loadSettings() {
    try {
        const data = await api('/models');
        const select = document.getElementById('model-select');
        select.innerHTML = data.models.map(m => 
            `<option value="${m.name}" ${m.name === data.current_model ? 'selected' : ''}>${m.name}</option>`
        ).join('');

        // Zeige aktuelles Modell
        const currentModelInfo = document.getElementById('current-model-info');
        if (currentModelInfo) {
            currentModelInfo.textContent = `Aktuell: ${data.current_model}`;
        }
    } catch (error) {
        console.error('Modelle laden fehlgeschlagen:', error);
    }
}

async function setModel(modelName) {
    try {
        const result = await api('/models', {
            method: 'POST',
            body: JSON.stringify({ model: modelName })
        });
        alert(`✅ Modell geändert auf: ${result.current_model}`);
        await loadSettings();
        await loadDashboard();
    } catch (error) {
        alert('❌ Fehler beim Ändern des Modells: ' + error.message);
    }
}

function initSettings() {
    document.getElementById('refresh-models-btn').addEventListener('click', loadSettings);
    
    // Modell-Änderung beim Select
    document.getElementById('model-select').addEventListener('change', async (e) => {
        const selectedModel = e.target.value;
        if (selectedModel) {
            await setModel(selectedModel);
        }
    });
}

// === Glossar ===
const glossaryState = {
    items: [],
    filter: 'all',
    searchQuery: '',
    editingId: null
};

async function loadGlossary() {
    const container = document.getElementById('glossary-list');
    
    // Zeige Skeleton Loading
    container.innerHTML = `
        <div class="glossary-skeleton">
            <div class="skeleton skeleton-title"></div>
            <div class="skeleton skeleton-text"></div>
            <div class="skeleton skeleton-text short"></div>
        </div>
        <div class="glossary-skeleton">
            <div class="skeleton skeleton-title"></div>
            <div class="skeleton skeleton-text"></div>
            <div class="skeleton skeleton-text short"></div>
        </div>
    `;

    try {
        glossaryState.items = await api('/glossary');
        renderGlossary();
    } catch (error) {
        // Falls API noch nicht existiert, zeige leere Liste
        glossaryState.items = [];
        renderGlossary();
    }
}

function renderGlossary() {
    const container = document.getElementById('glossary-list');
    let items = glossaryState.items;

    // Filter anwenden
    if (glossaryState.filter !== 'all') {
        items = items.filter(item => item.category === glossaryState.filter);
    }

    // Suche anwenden
    if (glossaryState.searchQuery) {
        const query = glossaryState.searchQuery.toLowerCase();
        items = items.filter(item => 
            item.term.toLowerCase().includes(query) ||
            item.definition.toLowerCase().includes(query) ||
            (item.details && item.details.toLowerCase().includes(query))
        );
    }

    if (items.length === 0) {
        container.innerHTML = `
            <div class="glossary-empty">
                <p>📖 Noch keine Begriffe im Glossar.</p>
                <p>Klicke auf "➕ Neuer Begriff" um zu starten.</p>
            </div>
        `;
        return;
    }

    container.innerHTML = items.map(item => `
        <div class="glossary-item" data-id="${item.id}" onclick="toggleGlossaryItem('${item.id}')">
            <div class="glossary-item-header">
                <span class="glossary-term">${item.term}</span>
                <span class="glossary-category ${item.category}">${getCategoryLabel(item.category)}</span>
            </div>
            <p class="glossary-definition">${item.definition}</p>
            <div class="glossary-details">
                ${item.details ? `<p>${formatMarkdownInline(item.details)}</p>` : ''}
                ${item.related && item.related.length > 0 ? `
                    <div class="glossary-related">
                        <strong>Verwandte Begriffe:</strong>
                        ${item.related.map(r => `<span class="related-tag" onclick="searchGlossaryTerm(event, '${r}')">${r}</span>`).join('')}
                    </div>
                ` : ''}
                <div class="glossary-actions">
                    <button class="btn btn-secondary" onclick="editGlossaryItem(event, '${item.id}')">✏️ Bearbeiten</button>
                    <button class="btn btn-secondary" onclick="deleteGlossaryItem(event, '${item.id}')">🗑️ Löschen</button>
                </div>
            </div>
        </div>
    `).join('');
}

function getCategoryLabel(category) {
    const labels = {
        'definition': 'Definition',
        'formula': 'Formel',
        'concept': 'Konzept',
        'abbreviation': 'Abkürzung',
        'other': 'Sonstiges'
    };
    return labels[category] || category;
}

function formatMarkdownInline(text) {
    if (typeof marked !== 'undefined') {
        return marked.parse(text);
    }
    return text
        .replace(/\*\*(.*?)\*\*/g, '<strong>$1</strong>')
        .replace(/\*(.*?)\*/g, '<em>$1</em>')
        .replace(/`([^`]+)`/g, '<code>$1</code>');
}

function toggleGlossaryItem(id) {
    const item = document.querySelector(`.glossary-item[data-id="${id}"]`);
    if (item) {
        item.classList.toggle('expanded');
    }
}

function searchGlossaryTerm(event, term) {
    event.stopPropagation();
    document.getElementById('glossary-search').value = term;
    glossaryState.searchQuery = term;
    renderGlossary();
}

function showGlossaryModal(item = null) {
    const modal = document.getElementById('glossary-modal');
    const title = document.getElementById('modal-title');
    const form = document.getElementById('glossary-form');

    if (item) {
        title.textContent = 'Begriff bearbeiten';
        glossaryState.editingId = item.id;
        document.getElementById('glossary-term').value = item.term;
        document.getElementById('glossary-category').value = item.category;
        document.getElementById('glossary-definition').value = item.definition;
        document.getElementById('glossary-details').value = item.details || '';
        document.getElementById('glossary-related').value = (item.related || []).join(', ');
    } else {
        title.textContent = 'Neuer Glossar-Eintrag';
        glossaryState.editingId = null;
        form.reset();
    }

    modal.classList.remove('hidden');
}

function hideGlossaryModal() {
    document.getElementById('glossary-modal').classList.add('hidden');
    glossaryState.editingId = null;
}

async function saveGlossaryItem(formData) {
    const item = {
        term: formData.term,
        category: formData.category,
        definition: formData.definition,
        details: formData.details,
        related: formData.related.split(',').map(r => r.trim()).filter(r => r)
    };

    try {
        if (glossaryState.editingId) {
            await api(`/glossary/${glossaryState.editingId}`, {
                method: 'PUT',
                body: JSON.stringify(item)
            });
        } else {
            await api('/glossary', {
                method: 'POST',
                body: JSON.stringify(item)
            });
        }
        hideGlossaryModal();
        await loadGlossary();
    } catch (error) {
        // Falls API noch nicht existiert, speichere lokal
        if (!glossaryState.editingId) {
            item.id = `local_${Date.now()}`;
            glossaryState.items.push(item);
        } else {
            const index = glossaryState.items.findIndex(i => i.id === glossaryState.editingId);
            if (index !== -1) {
                glossaryState.items[index] = { ...glossaryState.items[index], ...item };
            }
        }
        localStorage.setItem('glossary', JSON.stringify(glossaryState.items));
        hideGlossaryModal();
        renderGlossary();
    }
}

function editGlossaryItem(event, id) {
    event.stopPropagation();
    const item = glossaryState.items.find(i => i.id === id);
    if (item) {
        showGlossaryModal(item);
    }
}

async function deleteGlossaryItem(event, id) {
    event.stopPropagation();
    if (!confirm('Begriff wirklich löschen?')) return;

    try {
        await api(`/glossary/${id}`, { method: 'DELETE' });
        await loadGlossary();
    } catch (error) {
        // Falls API noch nicht existiert, lösche lokal
        glossaryState.items = glossaryState.items.filter(i => i.id !== id);
        localStorage.setItem('glossary', JSON.stringify(glossaryState.items));
        renderGlossary();
    }
}

function initGlossary() {
    // Lade lokale Daten falls vorhanden
    const localGlossary = localStorage.getItem('glossary');
    if (localGlossary) {
        glossaryState.items = JSON.parse(localGlossary);
    }

    // Suche
    document.getElementById('glossary-search').addEventListener('input', (e) => {
        glossaryState.searchQuery = e.target.value;
        renderGlossary();
    });

    // Filter Buttons
    document.querySelectorAll('.filter-btn').forEach(btn => {
        btn.addEventListener('click', () => {
            document.querySelectorAll('.filter-btn').forEach(b => b.classList.remove('active'));
            btn.classList.add('active');
            glossaryState.filter = btn.dataset.filter;
            renderGlossary();
        });
    });

    // Modal öffnen
    document.getElementById('add-glossary-btn').addEventListener('click', () => {
        showGlossaryModal();
    });

    // Modal schließen
    document.getElementById('close-modal').addEventListener('click', hideGlossaryModal);
    document.getElementById('cancel-glossary').addEventListener('click', hideGlossaryModal);

    // Formular absenden
    document.getElementById('glossary-form').addEventListener('submit', (e) => {
        e.preventDefault();
        const formData = {
            term: document.getElementById('glossary-term').value,
            category: document.getElementById('glossary-category').value,
            definition: document.getElementById('glossary-definition').value,
            details: document.getElementById('glossary-details').value,
            related: document.getElementById('glossary-related').value
        };
        saveGlossaryItem(formData);
    });

    // Modal schließen bei Klick außerhalb
    document.getElementById('glossary-modal').addEventListener('click', (e) => {
        if (e.target.id === 'glossary-modal') {
            hideGlossaryModal();
        }
    });
}

// === Initialize App ===
document.addEventListener('DOMContentLoaded', () => {
    initNavigation();
    initDocumentUpload();
    initStudyPlanForm();
    initLearnButtons();
    initQuizButtons();
    initChat();
    initSettings();
    initGlossary();
    loadSettingsUI(); // Quiz-Einstellungen laden
    
    // Load dashboard
    loadDashboard();
});

// Make functions available globally
window.deleteDocument = deleteDocument;
window.openTopic = openTopic;
window.selectLearnTopic = selectLearnTopic;
window.selectQuizTopic = selectQuizTopic;
window.toggleGlossaryItem = toggleGlossaryItem;
window.searchGlossaryTerm = searchGlossaryTerm;
window.editGlossaryItem = editGlossaryItem;
window.deleteGlossaryItem = deleteGlossaryItem;
