from __future__ import annotations

import html
import json

from .review import ReviewReport


def render_review_report_html(report: ReviewReport) -> str:
    payload = json.dumps(report.to_dict())
    escaped_title = html.escape(f"NeuroGraph Review · {report.pr_identifier}")
    return f"""<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{escaped_title}</title>
  <style>
    :root {{
      --paper: #f5efe3;
      --ink: #13212c;
      --accent: #cf5c36;
      --accent-2: #0f8b8d;
      --line: rgba(19, 33, 44, 0.12);
      --panel: rgba(255, 252, 247, 0.82);
      --shadow: 0 20px 60px rgba(19, 33, 44, 0.12);
      --radius: 22px;
      --font-display: "Georgia", "Iowan Old Style", serif;
      --font-body: "Segoe UI", "Helvetica Neue", sans-serif;
    }}
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0;
      min-height: 100vh;
      font-family: var(--font-body);
      color: var(--ink);
      background:
        radial-gradient(circle at top left, rgba(255, 185, 66, 0.35), transparent 28%),
        radial-gradient(circle at top right, rgba(15, 139, 141, 0.22), transparent 30%),
        linear-gradient(180deg, #f8f2e7 0%, #efe4d1 100%);
    }}
    .shell {{
      width: min(1400px, calc(100vw - 32px));
      margin: 24px auto;
      display: grid;
      grid-template-columns: 340px 1fr;
      gap: 20px;
    }}
    .hero, .sidebar, .panel {{
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: var(--radius);
      backdrop-filter: blur(14px);
      box-shadow: var(--shadow);
    }}
    .hero {{
      grid-column: 1 / -1;
      padding: 32px;
    }}
    .hero h1 {{
      margin: 0 0 10px;
      font-size: clamp(2rem, 4vw, 3.6rem);
      line-height: 0.94;
      font-family: var(--font-display);
      letter-spacing: -0.03em;
      max-width: 780px;
    }}
    .hero p {{
      max-width: 780px;
      margin: 0;
      font-size: 1.05rem;
      line-height: 1.6;
      color: rgba(19, 33, 44, 0.78);
    }}
    .stats {{
      margin-top: 24px;
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
      gap: 14px;
    }}
    .stat {{
      padding: 16px 18px;
      border-radius: 18px;
      background: rgba(255, 255, 255, 0.55);
      border: 1px solid rgba(19, 33, 44, 0.09);
    }}
    .stat strong {{
      display: block;
      font-size: 1.7rem;
      font-family: var(--font-display);
    }}
    .sidebar {{
      padding: 20px;
      position: sticky;
      top: 20px;
      align-self: start;
    }}
    .sidebar h2, .panel h2 {{
      margin: 0 0 12px;
      font-size: 0.88rem;
      letter-spacing: 0.12em;
      text-transform: uppercase;
      color: rgba(19, 33, 44, 0.64);
    }}
    .control {{
      margin-bottom: 18px;
    }}
    .search {{
      width: 100%;
      padding: 14px 16px;
      border-radius: 16px;
      border: 1px solid var(--line);
      background: rgba(255, 255, 255, 0.75);
      color: var(--ink);
      font-size: 0.95rem;
    }}
    .chips {{
      display: flex;
      flex-wrap: wrap;
      gap: 10px;
    }}
    .chip {{
      border: 1px solid var(--line);
      background: rgba(255, 255, 255, 0.6);
      color: var(--ink);
      padding: 10px 14px;
      border-radius: 999px;
      cursor: pointer;
    }}
    .chip.active {{
      background: linear-gradient(135deg, rgba(207, 92, 54, 0.18), rgba(15, 139, 141, 0.18));
      border-color: rgba(207, 92, 54, 0.5);
    }}
    .sidebar ul {{
      padding-left: 18px;
      margin: 0;
      color: rgba(19, 33, 44, 0.8);
      line-height: 1.6;
    }}
    .main {{
      display: grid;
      gap: 20px;
    }}
    .panel {{
      padding: 22px;
    }}
    .finding-list {{
      display: grid;
      gap: 14px;
    }}
    .finding {{
      border: 1px solid var(--line);
      border-radius: 18px;
      padding: 18px;
      background: rgba(255, 255, 255, 0.62);
      cursor: pointer;
    }}
    .finding.selected {{
      border-color: rgba(207, 92, 54, 0.45);
      box-shadow: 0 18px 40px rgba(207, 92, 54, 0.11);
    }}
    .finding-head {{
      display: flex;
      justify-content: space-between;
      gap: 12px;
      align-items: start;
      margin-bottom: 8px;
    }}
    .badge {{
      display: inline-flex;
      align-items: center;
      padding: 6px 10px;
      border-radius: 999px;
      font-size: 0.75rem;
      text-transform: uppercase;
      letter-spacing: 0.08em;
      font-weight: 700;
    }}
    .badge.high {{ background: rgba(180, 35, 24, 0.12); color: #b42318; }}
    .badge.medium {{ background: rgba(181, 71, 8, 0.12); color: #b54708; }}
    .badge.low {{ background: rgba(15, 139, 141, 0.12); color: var(--accent-2); }}
    .badge.info {{ background: rgba(21, 94, 239, 0.12); color: #155eef; }}
    .finding h3 {{
      margin: 0;
      font-size: 1.15rem;
      line-height: 1.25;
    }}
    .finding p {{
      margin: 0;
      color: rgba(19, 33, 44, 0.8);
      line-height: 1.55;
    }}
    .meta {{
      margin-top: 10px;
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
      color: rgba(19, 33, 44, 0.62);
      font-size: 0.85rem;
    }}
    .detail-grid, .file-list {{
      display: grid;
      gap: 10px;
    }}
    .detail-card, .file-row {{
      border-radius: 18px;
      padding: 16px;
      background: rgba(255, 255, 255, 0.58);
      border: 1px solid var(--line);
    }}
    .empty {{
      padding: 18px;
      border-radius: 18px;
      background: rgba(255, 255, 255, 0.45);
      border: 1px dashed var(--line);
      color: rgba(19, 33, 44, 0.62);
    }}
    @media (max-width: 1100px) {{
      .shell {{ grid-template-columns: 1fr; }}
      .sidebar {{ position: static; }}
    }}
  </style>
</head>
<body>
  <div class="shell">
    <section class="hero">
      <h1>NeuroGraph review for {html.escape(report.pr_identifier)}</h1>
      <p>Interactive review output generated from the live Rust graph engine. Filter the findings, inspect changed files, and read the evidence without leaving the report.</p>
      <div class="stats" id="stats"></div>
    </section>
    <aside class="sidebar">
      <div class="control">
        <h2>Search</h2>
        <input class="search" id="search" type="search" placeholder="Filter findings by title, file, node, or evidence">
      </div>
      <div class="control">
        <h2>Severity</h2>
        <div class="chips" id="severity-filters"></div>
      </div>
      <div class="control">
        <h2>Changed Files</h2>
        <div class="chips" id="file-filters"></div>
      </div>
      <div class="control">
        <h2>Warnings</h2>
        <ul id="warning-bullets"></ul>
      </div>
    </aside>
    <main class="main">
      <section class="panel">
        <h2>Findings</h2>
        <div id="finding-list" class="finding-list"></div>
      </section>
      <section class="panel">
        <h2>Finding Detail</h2>
        <div id="finding-detail" class="detail-grid"></div>
      </section>
      <section class="panel">
        <h2>Changed Files</h2>
        <div id="file-list" class="file-list"></div>
      </section>
    </main>
  </div>
  <script>
    const report = {payload};
    const severityOrder = ['all', 'high', 'medium', 'low', 'info'];
    const state = {{
      severity: 'all',
      file: 'all',
      query: '',
      selectedId: report.findings[0]?.id ?? null,
    }};

    const statsEl = document.getElementById('stats');
    const severityFiltersEl = document.getElementById('severity-filters');
    const fileFiltersEl = document.getElementById('file-filters');
    const warningBulletsEl = document.getElementById('warning-bullets');
    const findingListEl = document.getElementById('finding-list');
    const findingDetailEl = document.getElementById('finding-detail');
    const fileListEl = document.getElementById('file-list');
    const searchEl = document.getElementById('search');

    function escapeHtml(value) {{
      return String(value)
        .replaceAll('&', '&amp;')
        .replaceAll('<', '&lt;')
        .replaceAll('>', '&gt;')
        .replaceAll('"', '&quot;');
    }}

    function createChip(label, active, onClick) {{
      const button = document.createElement('button');
      button.className = `chip${{active ? ' active' : ''}}`;
      button.textContent = label;
      button.type = 'button';
      button.addEventListener('click', onClick);
      return button;
    }}

    function filteredFindings() {{
      return report.findings.filter((finding) => {{
        const matchesSeverity = state.severity === 'all' || finding.severity === state.severity;
        const matchesFile = state.file === 'all' || finding.file_path === state.file;
        const haystack = [
          finding.title,
          finding.summary,
          finding.file_path ?? '',
          finding.node_fqn ?? '',
          ...finding.evidence,
        ].join(' ').toLowerCase();
        const matchesQuery = !state.query || haystack.includes(state.query);
        return matchesSeverity && matchesFile && matchesQuery;
      }});
    }}

    function renderStats() {{
      const items = [
        ['Changed files', report.summary.changed_files],
        ['Changed nodes', report.summary.changed_nodes],
        ['Findings', report.summary.findings],
        ['Warnings', report.warnings.length],
      ];
      statsEl.innerHTML = '';
      for (const [label, value] of items) {{
        const card = document.createElement('div');
        card.className = 'stat';
        card.innerHTML = `<strong>${{value}}</strong><span>${{label}}</span>`;
        statsEl.appendChild(card);
      }}
    }}

    function renderFilters() {{
      severityFiltersEl.innerHTML = '';
      for (const severity of severityOrder) {{
        const count = severity === 'all'
          ? report.findings.length
          : report.findings.filter((finding) => finding.severity === severity).length;
        severityFiltersEl.appendChild(
          createChip(`${{severity.toUpperCase()}} · ${{count}}`, state.severity === severity, () => {{
            state.severity = severity;
            render();
          }})
        );
      }}

      const files = ['all', ...report.diff_analysis.changed_files.map((file) => file.file_path)];
      fileFiltersEl.innerHTML = '';
      for (const file of files) {{
        fileFiltersEl.appendChild(
          createChip(file === 'all' ? 'All files' : file, state.file === file, () => {{
            state.file = file;
            render();
          }})
        );
      }}
    }}

    function renderWarnings() {{
      warningBulletsEl.innerHTML = '';
      const warnings = report.warnings.length ? report.warnings : ['No review warnings were emitted.'];
      for (const warning of warnings) {{
        const li = document.createElement('li');
        li.textContent = warning;
        warningBulletsEl.appendChild(li);
      }}
    }}

    function renderFindings() {{
      const findings = filteredFindings();
      findingListEl.innerHTML = '';
      if (!findings.length) {{
        findingListEl.innerHTML = '<div class="empty">No findings match the current filters.</div>';
        findingDetailEl.innerHTML = '<div class="empty">Pick a finding to inspect the evidence and recommendation.</div>';
        return;
      }}

      if (!findings.some((finding) => finding.id === state.selectedId)) {{
        state.selectedId = findings[0].id;
      }}

      for (const finding of findings) {{
        const article = document.createElement('article');
        article.className = `finding${{finding.id === state.selectedId ? ' selected' : ''}}`;
        article.addEventListener('click', () => {{
          state.selectedId = finding.id;
          renderFindings();
          renderDetail();
        }});
        article.innerHTML = `
          <div class="finding-head">
            <h3>${{escapeHtml(finding.title)}}</h3>
            <span class="badge ${{finding.severity}}">${{finding.severity}}</span>
          </div>
          <p>${{escapeHtml(finding.summary)}}</p>
          <div class="meta">
            <span>${{escapeHtml(finding.kind)}}</span>
            <span>${{escapeHtml(finding.file_path ?? 'project-wide')}}</span>
            <span>${{escapeHtml(finding.node_fqn ?? 'n/a')}}</span>
          </div>
        `;
        findingListEl.appendChild(article);
      }}
    }}

    function renderDetail() {{
      const finding = report.findings.find((entry) => entry.id === state.selectedId);
      if (!finding) {{
        findingDetailEl.innerHTML = '<div class="empty">Pick a finding to inspect the evidence and recommendation.</div>';
        return;
      }}

      findingDetailEl.innerHTML = `
        <div class="detail-card">
          <h3>${{escapeHtml(finding.title)}}</h3>
          <p>${{escapeHtml(finding.summary)}}</p>
          <p><strong>Recommendation:</strong> ${{escapeHtml(finding.recommendation)}}</p>
          <p><strong>Node:</strong> ${{escapeHtml(finding.node_fqn ?? 'n/a')}}</p>
          <p><strong>File:</strong> ${{escapeHtml(finding.file_path ?? 'project-wide')}}</p>
          <p><strong>Confidence:</strong> ${{finding.confidence == null ? 'n/a' : finding.confidence.toFixed(2)}}</p>
          <ul>
            ${{finding.evidence.map((item) => `<li>${{escapeHtml(item)}}</li>`).join('') || '<li>No extra evidence recorded.</li>'}}
          </ul>
        </div>
      `;
    }}

    function renderFiles() {{
      fileListEl.innerHTML = '';
      if (!report.diff_analysis.changed_files.length) {{
        fileListEl.innerHTML = '<div class="empty">The diff analysis did not return any changed files.</div>';
        return;
      }}
      for (const file of report.diff_analysis.changed_files) {{
        const row = document.createElement('div');
        row.className = 'file-row';
        row.innerHTML = `
          <strong>${{escapeHtml(file.file_path)}}</strong>
          <div class="meta">
            <span>Added lines: ${{file.added_lines.length}}</span>
            <span>Removed lines: ${{file.removed_lines.length}}</span>
            <span>Mapped nodes: ${{file.changed_nodes.length}}</span>
          </div>
          <p>${{file.deleted_symbols.length ? `Deleted symbols: ${{escapeHtml(file.deleted_symbols.join(', '))}}` : 'No deleted symbols detected.'}}</p>
        `;
        fileListEl.appendChild(row);
      }}
    }}

    function render() {{
      renderFilters();
      renderWarnings();
      renderFindings();
      renderDetail();
      renderFiles();
    }}

    searchEl.addEventListener('input', (event) => {{
      state.query = event.target.value.trim().toLowerCase();
      render();
    }});

    renderStats();
    render();
  </script>
</body>
</html>
"""
