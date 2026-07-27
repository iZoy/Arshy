use arshy_lib::Result;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

/// Generates a premium HTML report from the JSON analytics output and opens it in the browser.
pub fn render_web_dashboard(report: &serde_json::Value) -> Result<()> {
    let html_content = generate_html(report)?;

    // Write to a temporary file
    let mut temp_dir = std::env::temp_dir();
    temp_dir.push("arshy-dashboard.html");

    let mut file = File::create(&temp_dir)?;
    file.write_all(html_content.as_bytes())?;

    println!("Web dashboard HTML report generated at: {}", temp_dir.display());
    println!("Opening in default browser...");

    open_in_browser(temp_dir)?;

    Ok(())
}

fn open_in_browser(path: PathBuf) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).status();
    }

    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).status();
    }

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(&["/C", "start"])
            .arg(path.to_string_lossy().to_string())
            .status();
    }

    Ok(())
}

fn generate_html(report: &serde_json::Value) -> Result<String> {
    let report_json = serde_json::to_string_pretty(report)?;

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Arshy — AI Command Center</title>
    
    <!-- Modern Typography -->
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Outfit:wght@300;400;500;600;700;800&family=Plus+Jakarta+Sans:wght@300;400;500;600;700;800&display=swap" rel="stylesheet">
    
    <!-- Tailwind CSS CDN -->
    <script src="https://cdn.tailwindcss.com"></script>
    
    <!-- Chart.js CDN -->
    <script src="https://cdn.jsdelivr.net/npm/chart.js"></script>

    <script>
        tailwind.config = {{
            theme: {{
                extend: {{
                    fontFamily: {{
                        sans: ['Plus Jakarta Sans', 'Inter', 'sans-serif'],
                        outfit: ['Outfit', 'sans-serif'],
                    }},
                    colors: {{
                        slate: {{
                            950: '#030712',
                        }}
                    }}
                }}
            }}
        }}
    </script>
    
    <style>
        body {{
            background: radial-gradient(circle at 50% -20%, #1e1b4b 0%, #030712 60%);
            min-height: 100vh;
        }}
        .glass-panel {{
            background: rgba(15, 23, 42, 0.45);
            backdrop-filter: blur(16px);
            -webkit-backdrop-filter: blur(16px);
            border: 1px solid rgba(255, 255, 255, 0.05);
            box-shadow: 0 8px 32px 0 rgba(0, 0, 0, 0.37);
        }}
        .glass-panel:hover {{
            border-color: rgba(255, 255, 255, 0.08);
            box-shadow: 0 8px 32px 0 rgba(168, 85, 247, 0.05);
            transition: all 0.3s ease;
        }}
        .text-glow {{
            text-shadow: 0 0 20px rgba(168, 85, 247, 0.4);
        }}
        .active-tab {{
            background: linear-gradient(135deg, #a855f7 0%, #ec4899 100%);
            box-shadow: 0 0 15px rgba(168, 85, 247, 0.3);
            border-color: transparent;
        }}
    </style>
</head>
<body class="text-slate-100 font-sans antialiased pb-20">
    <div class="max-w-7xl mx-auto px-6 pt-10">
        
        <!-- Header -->
        <header class="flex flex-col md:flex-row justify-between items-start md:items-center mb-10 pb-8 border-b border-slate-800/60">
            <div>
                <div class="flex items-center gap-3 mb-2">
                    <span class="text-3xl font-extrabold tracking-tight bg-gradient-to-r from-purple-400 via-pink-400 to-indigo-400 bg-clip-text text-transparent font-outfit" data-i18n="title">
                        ARSHY ANALYTICS
                    </span>
                    <span class="px-2.5 py-1 text-xs font-semibold text-purple-300 bg-purple-950/50 border border-purple-500/30 rounded-full flex items-center gap-1.5">
                        <span class="h-2 w-2 rounded-full bg-purple-500 animate-pulse"></span>
                        <span data-i18n="session">Active Session</span>
                    </span>
                </div>
                <p class="text-slate-400 text-sm" data-i18n="subtitle">Semantic shell execution optimization layer for AI Agents</p>
            </div>
            <div class="mt-4 md:mt-0 flex flex-col items-start md:items-end gap-3">
                <div class="text-left md:text-right">
                    <span class="text-xs text-slate-500 block font-semibold tracking-wider" data-i18n="generated">GENERATED AT</span>
                    <span class="text-sm font-mono text-slate-300 font-semibold" id="gen-date">Loading...</span>
                </div>
                <button id="lang-btn" onclick="toggleLanguage()" class="px-3.5 py-1 text-xs font-bold bg-slate-900/80 hover:bg-slate-800 text-purple-300 hover:text-purple-200 border border-purple-500/30 rounded-lg shadow-md transition-all active:scale-95">
                    中文
                </button>
            </div>
        </header>

        <!-- Main Navigation Tabs -->
        <div class="flex gap-2 mb-8 bg-slate-900/60 p-1.5 rounded-xl border border-slate-800 max-w-md">
            <button onclick="switchTab('overview')" id="tab-overview" class="flex-1 py-2 px-4 rounded-lg text-sm font-semibold border border-transparent text-slate-300 transition-all hover:text-white active-tab" data-i18n="tabOverview">
                Overview
            </button>
            <button onclick="switchTab('parsers')" id="tab-parsers" class="flex-1 py-2 px-4 rounded-lg text-sm font-semibold border border-transparent text-slate-400 transition-all hover:text-white" data-i18n="tabParsers">
                Parser Analysis
            </button>
            <button onclick="switchTab('json')" id="tab-json" class="flex-1 py-2 px-4 rounded-lg text-sm font-semibold border border-transparent text-slate-400 transition-all hover:text-white" data-i18n="tabJson">
                Raw JSON
            </button>
        </div>

        <!-- TAB CONTENT: OVERVIEW -->
        <div id="content-overview" class="tab-content space-y-8">
            
            <!-- KPI Grid -->
            <div class="grid grid-cols-1 md:grid-cols-4 gap-6">
                <!-- KPI 1 -->
                <div class="glass-panel rounded-2xl p-6 relative overflow-hidden group">
                    <div class="absolute -right-4 -top-4 w-24 h-24 bg-purple-500/5 rounded-full group-hover:scale-125 transition-transform duration-500"></div>
                    <span class="text-xs font-semibold text-slate-400 tracking-wider uppercase block mb-1" data-i18n="kpiSavings">Token Savings Rate</span>
                    <div class="flex items-baseline gap-2">
                        <span class="text-4xl font-extrabold text-glow text-purple-400 font-outfit" id="metric-savings">0%</span>
                        <span class="text-xs text-emerald-400 font-medium font-mono" id="metric-savings-desc">Filtered</span>
                    </div>
                    <div class="w-full bg-slate-800 h-1.5 rounded-full mt-4 overflow-hidden">
                        <div class="bg-gradient-to-r from-purple-500 to-pink-500 h-full rounded-full" id="metric-savings-bar" style="width: 0%"></div>
                    </div>
                </div>

                <!-- KPI 2 -->
                <div class="glass-panel rounded-2xl p-6 relative overflow-hidden group">
                    <div class="absolute -right-4 -top-4 w-24 h-24 bg-cyan-500/5 rounded-full group-hover:scale-125 transition-transform duration-500"></div>
                    <span class="text-xs font-semibold text-slate-400 tracking-wider uppercase block mb-1" data-i18n="kpiTasks">Total Tasks Ran</span>
                    <div class="flex items-baseline gap-2">
                        <span class="text-4xl font-extrabold text-cyan-400 font-outfit" id="metric-tasks">0</span>
                    </div>
                    <p class="text-xs text-slate-400 mt-4" id="metric-tasks-desc">Completed daemon cycles</p>
                </div>

                <!-- KPI 3 -->
                <div class="glass-panel rounded-2xl p-6 relative overflow-hidden group">
                    <div class="absolute -right-4 -top-4 w-24 h-24 bg-amber-500/5 rounded-full group-hover:scale-125 transition-transform duration-500"></div>
                    <span class="text-xs font-semibold text-slate-400 tracking-wider uppercase block mb-1" data-i18n="kpiNoise">Noise Reduction</span>
                    <div class="flex items-baseline gap-2">
                        <span class="text-4xl font-extrabold text-amber-400 font-outfit" id="metric-noise">0%</span>
                    </div>
                    <p class="text-xs text-slate-400 mt-4" id="metric-noise-desc">Filtered shell outputs</p>
                </div>

                <!-- KPI 4 -->
                <div class="glass-panel rounded-2xl p-6 relative overflow-hidden group">
                    <div class="absolute -right-4 -top-4 w-24 h-24 bg-emerald-500/5 rounded-full group-hover:scale-125 transition-transform duration-500"></div>
                    <span class="text-xs font-semibold text-slate-400 tracking-wider uppercase block mb-1" data-i18n="kpiDensity">Structured Rate</span>
                    <div class="flex items-baseline gap-2">
                        <span class="text-4xl font-extrabold text-emerald-400 font-outfit" id="metric-density">0%</span>
                    </div>
                    <p class="text-xs text-slate-400 mt-4" id="metric-density-desc">Enriched diagnostics</p>
                </div>
            </div>

            <!-- Charts Section -->
            <div class="grid grid-cols-1 lg:grid-cols-3 gap-6">
                <!-- Token Efficiency Chart -->
                <div class="glass-panel rounded-2xl p-6 lg:col-span-2 flex flex-col justify-between">
                    <div>
                        <h3 class="text-lg font-bold tracking-tight text-slate-100 font-outfit mb-1" data-i18n="volTitle">Token Volume Comparison</h3>
                        <p class="text-xs text-slate-400 mb-6" data-i18n="volDesc">Compare Raw output string sizes vs Arshy structured events</p>
                    </div>
                    <div class="h-64 flex items-center justify-center">
                        <canvas id="efficiencyChart" class="w-full h-full"></canvas>
                    </div>
                </div>

                <!-- Information Density Chart -->
                <div class="glass-panel rounded-2xl p-6 flex flex-col justify-between">
                    <div>
                        <h3 class="text-lg font-bold tracking-tight text-slate-100 font-outfit mb-1" data-i18n="densityTitle">Information Density</h3>
                        <p class="text-xs text-slate-400 mb-6" data-i18n="densityDesc">Proportion of events containing rich semantic contexts</p>
                    </div>
                    <div class="h-60 flex items-center justify-center">
                        <canvas id="densityChart"></canvas>
                    </div>
                </div>
            </div>

            <!-- Bottom Row: Temporal and Top Commands -->
            <div class="grid grid-cols-1 lg:grid-cols-2 gap-6">
                <!-- Timeline chart -->
                <div class="glass-panel rounded-2xl p-6">
                    <h3 class="text-lg font-bold tracking-tight text-slate-100 font-outfit mb-4" data-i18n="timelineTitle">Activity Timeline</h3>
                    <div class="h-64">
                        <canvas id="timelineChart"></canvas>
                    </div>
                </div>

                <!-- Retries table -->
                <div class="glass-panel rounded-2xl p-6 flex flex-col justify-between">
                    <div>
                        <h3 class="text-lg font-bold tracking-tight text-slate-100 font-outfit mb-1" data-i18n="retriesTitle">Top Retried Commands</h3>
                        <p class="text-xs text-slate-400 mb-4" data-i18n="retriesDesc">Command blocks that repeatedly failed and triggered retries</p>
                    </div>
                    <div class="overflow-x-auto">
                        <table class="w-full text-left border-collapse">
                            <thead>
                                <tr class="border-b border-slate-800 text-slate-400 text-xs uppercase tracking-wider font-semibold">
                                    <th class="py-2.5 pb-3" data-i18n="tblCmd">Command</th>
                                    <th class="py-2.5 pb-3 text-right" data-i18n="tblRuns">Runs</th>
                                </tr>
                            </thead>
                            <tbody id="retries-table-body" class="text-sm font-mono text-slate-300">
                                <!-- Populated dynamically -->
                            </tbody>
                        </table>
                    </div>
                </div>
            </div>

        </div>

        <!-- TAB CONTENT: PARSERS -->
        <div id="content-parsers" class="tab-content hidden space-y-8">
            <div class="glass-panel rounded-2xl p-6">
                <h3 class="text-lg font-bold tracking-tight text-slate-100 font-outfit mb-1" data-i18n="parsersTitle">Parser Performance Breakdown</h3>
                <p class="text-xs text-slate-400 mb-6" data-i18n="parsersDesc">Details on structured field matching rates and skipped line logs</p>
                
                <div class="grid grid-cols-1 md:grid-cols-3 gap-6 mb-8" id="parser-stats-grid">
                    <!-- Dynamic Parser KPI metrics -->
                </div>
                
                <div class="overflow-x-auto">
                    <table class="w-full text-left border-collapse">
                        <thead>
                            <tr class="border-b border-slate-800 text-slate-400 text-xs uppercase tracking-wider font-semibold">
                                <th class="py-3" data-i18n="tblGroup">Parser Group</th>
                                <th class="py-3 text-center" data-i18n="tblAvgFields">Avg Fields / Event</th>
                                <th class="py-3 text-center" data-i18n="tblDiagCode">Diagnostics with Code</th>
                                <th class="py-3 text-center" data-i18n="tblDiagLoc">Diagnostics with Location</th>
                            </tr>
                        </thead>
                        <tbody id="parsers-table-body" class="text-sm text-slate-300">
                            <!-- Populated dynamically -->
                        </tbody>
                    </table>
                </div>
            </div>
        </div>

        <!-- TAB CONTENT: JSON -->
        <div id="content-json" class="tab-content hidden">
            <div class="glass-panel rounded-2xl p-6">
                <div class="flex justify-between items-center mb-4">
                    <h3 class="text-lg font-bold tracking-tight text-slate-100 font-outfit" data-i18n="rawTitle">Complete Raw Report</h3>
                    <button onclick="copyJson()" class="px-4 py-1.5 text-xs font-semibold bg-slate-800 hover:bg-slate-700 text-slate-200 border border-slate-700 rounded-lg transition-all" id="copy-btn" data-i18n="copyBtn">
                        Copy Code
                    </button>
                </div>
                <pre class="bg-slate-950/60 p-6 rounded-xl border border-slate-900 text-xs font-mono text-emerald-400 overflow-x-auto max-h-[600px]" id="raw-json-block"></pre>
            </div>
        </div>

    </div>

    <!-- Inject the raw report data -->
    <script>
        const report = {report_json};

        // i18n mapping definitions
        const i18n = {{
            en: {{
                title: "ARSHY ANALYTICS",
                subtitle: "Semantic shell execution optimization layer for AI Agents",
                session: "Active Session",
                generated: "GENERATED AT",
                tabOverview: "Overview",
                tabParsers: "Parser Analysis",
                tabJson: "Raw JSON",
                kpiSavings: "Token Savings Rate",
                kpiTasks: "Total Tasks Ran",
                kpiNoise: "Noise Reduction",
                kpiDensity: "Structured Rate",
                volTitle: "Token Volume Comparison",
                volDesc: "Compare Raw output string sizes vs Arshy structured events",
                densityTitle: "Information Density",
                densityDesc: "Proportion of events containing rich semantic contexts",
                timelineTitle: "Activity Timeline",
                retriesTitle: "Top Retried Commands",
                retriesDesc: "Command blocks that repeatedly failed and triggered retries",
                parsersTitle: "Parser Performance Breakdown",
                parsersDesc: "Details on structured field matching rates and skipped line logs",
                rawTitle: "Complete Raw Report",
                copyBtn: "Copy Code",
                lblTasks: "Tasks Executed",
                lblEvents: "Events Parsed",
                lblWithLoc: "With Files/Lines",
                lblWithCode: "With Error Codes",
                lblWithCtx: "With Context (±3 Lines)",
                lblRawLogs: "Raw Logs",
                tblCmd: "Command",
                tblRuns: "Runs",
                tblGroup: "Parser Group",
                tblAvgFields: "Avg Fields / Event",
                tblDiagCode: "Diagnostics with Code",
                tblDiagLoc: "Diagnostics with Location",
                cardAvgFields: "Average Fields/Event",
                cardWithLoc: "Events with locations",
                cardWithCode: "Events with codes",
                noRetries: "No command retries recorded yet",
                sizeBytes: "Size in Bytes",
                copySuccess: "Raw JSON report copied to clipboard!"
            }},
            zh: {{
                title: "ARSHY 数据分析",
                subtitle: "面向 AI Agent 的语义 Shell 执行优化层",
                session: "运行会话激活",
                generated: "报告生成时间",
                tabOverview: "数据概览",
                tabParsers: "解析器分析",
                tabJson: "原始 JSON",
                kpiSavings: "Token 节省率",
                kpiTasks: "已执行任务数",
                kpiNoise: "日志噪音过滤率",
                kpiDensity: "结构化事件比例",
                volTitle: "Token 体积对比",
                volDesc: "原始编译器日志输出体积与 Arshy 结构化事件体积的对比",
                densityTitle: "信息丰富密度",
                densityDesc: "包含丰富语义上下文（源码、错误码等）的事件占比",
                timelineTitle: "活动时间线",
                retriesTitle: "高频重试命令",
                retriesDesc: "本地开发中因高频失败导致 Agent 重试执行的命令排行",
                parsersTitle: "解析器性能细分",
                parsersDesc: "结构化字段匹配率与噪音过滤行的统计明细",
                rawTitle: "完整原始数据报告",
                copyBtn: "复制 JSON",
                lblTasks: "已执行任务",
                lblEvents: "已解析事件",
                lblWithLoc: "包含文件/行号",
                lblWithCode: "包含错误代码",
                lblWithCtx: "包含源码上下文 (±3行)",
                lblRawLogs: "普通日志",
                tblCmd: "执行命令",
                tblRuns: "重试次数",
                tblGroup: "解析器分组",
                tblAvgFields: "平均字段数 / 事件",
                tblDiagCode: "包含代码的诊断数",
                tblDiagLoc: "包含位置的诊断数",
                cardAvgFields: "平均字段数 / 事件",
                cardWithLoc: "包含位置信息的事件",
                cardWithCode: "包含错误码的事件",
                noRetries: "暂无命令重试记录",
                sizeBytes: "体积 (字节)",
                copySuccess: "原始 JSON 报告已成功复制到剪贴板！"
            }}
        }};

        let currentLang = 'en';
        function toggleLanguage() {{
            currentLang = currentLang === 'en' ? 'zh' : 'en';
            document.getElementById('lang-btn').innerText = currentLang === 'en' ? '中文' : 'English';
            updateLanguage(currentLang);
        }}

        function updateLanguage(lang) {{
            document.querySelectorAll('[data-i18n]').forEach(el => {{
                const key = el.getAttribute('data-i18n');
                if (i18n[lang][key]) {{
                    el.innerText = i18n[lang][key];
                }}
            }});
            
            // Dynamically refresh table header and card layouts
            const retriesTbody = document.getElementById('retries-table-body');
            if (!report.command_patterns.top_retried || report.command_patterns.top_retried.length === 0) {{
                retriesTbody.innerHTML = `<tr><td colspan="2" class="py-6 text-center text-slate-500">${{i18n[lang].noRetries}}</td></tr>`;
            }}

            // Dynamic Chart.js text refreshes
            if (efficiencyChartInst) {{
                efficiencyChartInst.data.labels = ['Raw Output Logs', i18n[lang].lblEvents];
                efficiencyChartInst.data.datasets[0].label = i18n[lang].sizeBytes;
                efficiencyChartInst.update();
            }}
            if (densityChartInst) {{
                densityChartInst.data.labels = [
                    i18n[lang].lblWithLoc,
                    i18n[lang].lblWithCode,
                    i18n[lang].lblWithCtx,
                    i18n[lang].lblRawLogs
                ];
                densityChartInst.update();
            }}
            if (timelineChartInst) {{
                timelineChartInst.data.datasets[0].label = i18n[lang].lblTasks;
                timelineChartInst.data.datasets[1].label = i18n[lang].lblEvents;
                timelineChartInst.update();
            }}
        }}

        // Populate basic metrics
        document.getElementById('gen-date').innerText = report.generated_at || new Date().toLocaleString();
        
        // Overview Metric card assignments
        const savingsRate = Math.round(report.token_efficiency.estimated_token_savings_pct || 0);
        document.getElementById('metric-savings').innerText = savingsRate + '%';
        document.getElementById('metric-savings-bar').style.width = savingsRate + '%';
        document.getElementById('metric-savings-desc').innerText = report.token_efficiency.total_structured_bytes ? 
            `Saved ${{Math.round(report.token_efficiency.total_raw_output_bytes - report.token_efficiency.total_structured_bytes)}} bytes` : 'Filtered';

        document.getElementById('metric-tasks').innerText = report.summary.total_tasks || 0;
        document.getElementById('metric-tasks-desc').innerText = `${{report.summary.total_errors || 0}} compilation errors flagged`;

        const noiseRate = Math.round(report.token_efficiency.noise_pct || 0);
        document.getElementById('metric-noise').innerText = noiseRate + '%';
        document.getElementById('metric-noise-desc').innerText = `${{report.token_efficiency.agent_skipped_events || 0}} noise logs discarded`;

        const densityRate = Math.round(report.information_density.structured_event_pct || 0);
        document.getElementById('metric-density').innerText = densityRate + '%';
        document.getElementById('metric-density-desc').innerText = `${{report.information_density.events_with_location || 0}} events with files/lines`;

        // Render Top Retried table
        const retriesTbody = document.getElementById('retries-table-body');
        if (report.command_patterns.top_retried && report.command_patterns.top_retried.length > 0) {{
            report.command_patterns.top_retried.forEach(([cmd, count]) => {{
                const row = document.createElement('tr');
                row.className = 'border-b border-slate-800/40 hover:bg-slate-900/10 transition-all';
                row.innerHTML = `
                    <td class="py-3 text-slate-200 truncate max-w-lg">${{cmd}}</td>
                    <td class="py-3 text-right text-purple-400 font-bold">${{count}}</td>
                `;
                retriesTbody.appendChild(row);
            }});
        }} else {{
            retriesTbody.innerHTML = `<tr><td colspan="2" class="py-6 text-center text-slate-500">No command retries recorded yet</td></tr>`;
        }}

        // Render JSON Block
        document.getElementById('raw-json-block').textContent = JSON.stringify(report, null, 2);

        // Chart.js - Global references
        let efficiencyChartInst, densityChartInst, timelineChartInst;

        // Chart.js - Token Volume Chart
        const ctxEff = document.getElementById('efficiencyChart').getContext('2d');
        efficiencyChartInst = new Chart(ctxEff, {{
            type: 'bar',
            data: {{
                labels: ['Raw Output Logs', 'Structured Events'],
                datasets: [{{
                    label: 'Size in Bytes',
                    data: [
                        report.token_efficiency.total_raw_output_bytes || 0,
                        report.token_efficiency.total_structured_bytes || 0
                    ],
                    backgroundColor: [
                        'rgba(168, 85, 247, 0.45)', // Purple
                        'rgba(20, 184, 166, 0.45)'  // Teal
                    ],
                    borderColor: [
                        'rgb(168, 85, 247)',
                        'rgb(20, 184, 166)'
                    ],
                    borderWidth: 2,
                    borderRadius: 8
                }}]
            }},
            options: {{
                responsive: true,
                maintainAspectRatio: false,
                plugins: {{
                    legend: {{ display: false }}
                }},
                scales: {{
                    y: {{
                        grid: {{ color: 'rgba(255, 255, 255, 0.05)' }},
                        ticks: {{ color: '#94a3b8' }}
                    }},
                    x: {{
                        grid: {{ display: false }},
                        ticks: {{ color: '#94a3b8' }}
                    }}
                }}
            }}
        }});

        // Chart.js - Information Density Doughnut
        const ctxDen = document.getElementById('densityChart').getContext('2d');
        densityChartInst = new Chart(ctxDen, {{
            type: 'doughnut',
            data: {{
                labels: ['With Files/Lines', 'With Error Codes', 'With Context (±3 Lines)', 'Raw Logs'],
                datasets: [{{
                    data: [
                        report.information_density.events_with_location || 0,
                        report.information_density.events_with_code || 0,
                        report.information_density.events_with_context || 0,
                        (report.summary.total_events || 0) - (report.information_density.events_with_location || 0)
                    ],
                    backgroundColor: [
                        'rgba(20, 184, 166, 0.65)',  // Teal
                        'rgba(59, 130, 246, 0.65)',  // Blue
                        'rgba(168, 85, 247, 0.65)',  // Purple
                        'rgba(100, 116, 139, 0.3)'   // Slate
                    ],
                    borderColor: [
                        'rgb(20, 184, 166)',
                        'rgb(59, 130, 246)',
                        'rgb(168, 85, 247)',
                        'rgb(100, 116, 139)'
                    ],
                    borderWidth: 1.5
                }}]
            }},
            options: {{
                responsive: true,
                maintainAspectRatio: false,
                plugins: {{
                    legend: {{
                        position: 'bottom',
                        labels: {{
                            color: '#94a3b8',
                            boxWidth: 12,
                            padding: 15,
                            font: {{ family: 'Plus Jakarta Sans', size: 11 }}
                        }}
                    }}
                }}
            }}
        }});

        // Chart.js - Timeline Chart
        const ctxTime = document.getElementById('timelineChart').getContext('2d');
        timelineChartInst = new Chart(ctxTime, {{
            type: 'line',
            data: {{
                labels: ['Start', 'End'],
                datasets: [
                    {{
                        label: 'Tasks Executed',
                        data: [0, report.summary.total_tasks || 0],
                        borderColor: 'rgb(168, 85, 247)',
                        backgroundColor: 'rgba(168, 85, 247, 0.05)',
                        tension: 0.3,
                        fill: true
                    }},
                    {{
                        label: 'Events Parsed',
                        data: [0, report.summary.total_events || 0],
                        borderColor: 'rgb(20, 184, 166)',
                        backgroundColor: 'rgba(20, 184, 166, 0.05)',
                        tension: 0.3,
                        fill: true
                    }}
                ]
            }},
            options: {{
                responsive: true,
                maintainAspectRatio: false,
                plugins: {{
                    legend: {{
                        labels: {{ color: '#94a3b8' }}
                    }}
                }},
                scales: {{
                    y: {{
                        grid: {{ color: 'rgba(255, 255, 255, 0.05)' }},
                        ticks: {{ color: '#94a3b8' }}
                    }},
                    x: {{
                        grid: {{ display: false }},
                        ticks: {{ color: '#94a3b8' }}
                    }}
                }}
            }}
        }});

        // Parse parser table details dynamically
        const parserGrid = document.getElementById('parser-stats-grid');
        const parserTable = document.getElementById('parsers-table-body');
        
        // Render some static card metrics for Parsers
        parserGrid.innerHTML = `
            <div class="glass-panel rounded-xl p-5">
                <span class="text-xs text-slate-400 block mb-1" data-i18n="cardAvgFields">Average Fields/Event</span>
                <span class="text-3xl font-extrabold text-teal-400 font-outfit">${{report.information_density.avg_fields_per_event.toFixed(1)}}</span>
            </div>
            <div class="glass-panel rounded-xl p-5">
                <span class="text-xs text-slate-400 block mb-1" data-i18n="cardWithLoc">Events with locations</span>
                <span class="text-3xl font-extrabold text-blue-400 font-outfit">${{report.information_density.events_with_location}}</span>
            </div>
            <div class="glass-panel rounded-xl p-5">
                <span class="text-xs text-slate-400 block mb-1" data-i18n="cardWithCode">Events with codes</span>
                <span class="text-3xl font-extrabold text-purple-400 font-outfit">${{report.information_density.events_with_code}}</span>
            </div>
        `;

        // We can add a row for general stats
        const row = document.createElement('tr');
        row.className = 'border-b border-slate-800/40 hover:bg-slate-900/10 transition-all';
        row.innerHTML = `
            <td class="py-3.5 font-semibold text-slate-200">Global Analytics</td>
            <td class="py-3.5 text-center font-mono">${{report.information_density.avg_fields_per_event.toFixed(2)}}</td>
            <td class="py-3.5 text-center font-mono text-purple-400 font-bold">${{report.information_density.events_with_code}}</td>
            <td class="py-3.5 text-center font-mono text-blue-400 font-bold">${{report.information_density.events_with_location}}</td>
        `;
        parserTable.appendChild(row);

        // Tab switcher function
        function switchTab(tab) {{
            document.querySelectorAll('.tab-content').forEach(c => c.classList.add('hidden'));
            document.querySelectorAll('header + div button').forEach(b => b.classList.remove('active-tab'));
            
            document.getElementById(`content-${{tab}}`).classList.remove('hidden');
            document.getElementById(`tab-${{tab}}`).classList.add('active-tab');
        }}

        // Copy JSON utility
        function copyJson() {{
            navigator.clipboard.writeText(JSON.stringify(report, null, 2));
            alert(i18n[currentLang].copySuccess);
        }}
    </script>
</body>
</html>"#,
        report_json = report_json
    );

    Ok(html)
}
