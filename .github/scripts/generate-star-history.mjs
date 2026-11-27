#!/usr/bin/env node
// Generate a self-hosted cumulative GitHub stars SVG chart for README.md (zero external deps).

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

const DAY_MS = 86_400_000;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function parseArgs(argv) {
  let repo = "aphelion31/saturn";
  let output = "docs/images/star-history.svg";
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === "--repo" && argv[i + 1]) {
      repo = argv[++i];
    } else if (argv[i] === "--output" && argv[i + 1]) {
      output = argv[++i];
    }
  }
  return { repo, output };
}

function utcDay(date) {
  return Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate());
}

function weekStartMs(dayMs) {
  const dow = (new Date(dayMs).getUTCDay() + 6) % 7; // Monday = 0
  return dayMs - dow * DAY_MS;
}

function formatMonthDay(ms) {
  const d = new Date(ms);
  return `${MONTHS[d.getUTCMonth()]} ${d.getUTCDate()}`;
}

async function fetchStarDates(repo, token) {
  let url = `https://api.github.com/repos/${repo}/stargazers?per_page=100`;
  const dates = [];
  while (url) {
    const res = await fetch(url, {
      headers: {
        Accept: "application/vnd.github.star+json",
        Authorization: `Bearer ${token}`,
        "User-Agent": "saturn-star-history",
        "X-GitHub-Api-Version": "2022-11-28",
      },
    });
    if (!res.ok) {
      throw new Error(`GitHub API ${res.status}: ${await res.text()}`);
    }
    const items = await res.json();
    for (const item of items) {
      dates.push(utcDay(new Date(item.starred_at)));
    }
    const link = res.headers.get("link") || "";
    const nextMatch = link.match(/<([^>]+)>;\s*rel="next"/);
    url = nextMatch ? nextMatch[1] : "";
  }
  return dates;
}

function renderSvg(repo, starDays, todayMs = utcDay(new Date())) {
  starDays.sort((a, b) => a - b);
  const currentWeek = weekStartMs(todayMs);
  const weeks = Array.from({ length: 26 }, (_, i) => currentWeek - (25 - i) * 7 * DAY_MS);

  const values = weeks.map((w) => {
    const end = Math.min(w + 6 * DAY_MS, todayMs);
    return starDays.filter((d) => d <= end).length;
  });
  const weekly = weeks.map((w) => {
    const end = Math.min(w + 6 * DAY_MS, todayMs);
    return starDays.filter((d) => d >= w && d <= end).length;
  });

  const width = 800;
  const height = 420;
  const left = 68;
  const right = 24;
  const top = 78;
  const bottom = 64;
  const plotW = width - left - right;
  const plotH = height - top - bottom;

  const latest = values[values.length - 1];
  const magnitude = latest > 0 ? 10 ** Math.max(0, String(latest).length - 2) : 1;
  const step = latest > 0 ? Math.max(magnitude, Math.ceil(latest / 4 / magnitude) * magnitude) : 1;
  const gridMax = Math.max(step * 4, 4);

  const x = (i) => left + (i * plotW) / (weeks.length - 1);
  const y = (val) => top + (1 - val / gridMax) * plotH;

  const yTicks = Array.from({ length: 5 }, (_, i) => {
    const val = step * i;
    const yy = y(val);
    const label = val >= 1000 ? `${Number((val / 1000).toFixed(2))}k` : String(val);
    return `<line x1="${left}" y1="${yy.toFixed(1)}" x2="${width - right}" y2="${yy.toFixed(1)}" class="grid"/><text x="${left - 12}" y="${(yy + 5).toFixed(1)}" text-anchor="end">${label}</text>`;
  }).join("");

  const points = values.map((v, i) => `${x(i).toFixed(1)},${y(v).toFixed(1)}`).join(" ");
  const area = `${left},${(top + plotH).toFixed(1)} ${points} ${width - right},${(top + plotH).toFixed(1)}`;

  const xTicks = [];
  const dots = [];
  weeks.forEach((w, i) => {
    const label = formatMonthDay(w);
    if (i % 4 === 0 || i === weeks.length - 1) {
      xTicks.push(`<text x="${x(i).toFixed(1)}" y="${height - 30}" text-anchor="middle">${label}</text>`);
    }
    const isCurrent = w === currentWeek;
    const cls = isCurrent ? "point current" : "point";
    const r = isCurrent ? 5 : 3;
    dots.push(
      `<circle class="${cls}" cx="${x(i).toFixed(1)}" cy="${y(values[i]).toFixed(1)}" r="${r}">` +
        `<title>${label}: ${values[i].toLocaleString("en-US")} total stars (+${weekly[i].toLocaleString("en-US")} that week)</title></circle>`,
    );
  });

  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" role="img" aria-labelledby="title desc">
<title id="title">${repo} cumulative GitHub stars over time</title>
<desc id="desc">Cumulative GitHub stars sampled weekly over the last 26 weeks, ending at ${latest.toLocaleString("en-US")} stars.</desc>
<style>
  :root { color-scheme: light dark; }
  .bg { fill: #fff; } text { fill: #57606a; font: 13px -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif; }
  .grid { stroke: #d8dee4; stroke-width: 1; }
  .area { fill: #2f81f7; fill-opacity: .14; } .line { fill: none; stroke: #2f81f7; stroke-width: 3; stroke-linejoin: round; stroke-linecap: round; }
  .point { fill: #fff; stroke: #2f81f7; stroke-width: 2; } .point.current { fill: #2f81f7; stroke: #fff; stroke-width: 2; }
  .heading { fill: #24292f; font-size: 17px; font-weight: 600; } .metric { fill: #24292f; font-size: 18px; font-weight: 600; }
  @media (prefers-color-scheme: dark) { .bg { fill: #0d1117; } text { fill: #8b949e; } .grid { stroke: #30363d; } .area { fill: #58a6ff; fill-opacity: .16; } .line { stroke: #58a6ff; } .point { fill: #0d1117; stroke: #58a6ff; } .point.current { fill: #58a6ff; stroke: #0d1117; } .heading,.metric { fill: #f0f6fc; } }
</style>
<rect class="bg" width="100%" height="100%" rx="6"/>
<text class="heading" x="${left}" y="28">GitHub stars over time</text>
<text x="${left}" y="51">Cumulative stars · weekly sampling · last 26 weeks</text>
<text class="metric" x="${width - right}" y="28" text-anchor="end">${latest.toLocaleString("en-US")} stars</text>
<text x="${width - right}" y="51" text-anchor="end">+${weekly[weekly.length - 1].toLocaleString("en-US")} this week so far</text>
${yTicks}
<polygon class="area" points="${area}"/>
<polyline class="line" points="${points}"/>
${dots.join("")}
${xTicks.join("")}
</svg>
`;
}

const { repo, output } = parseArgs(process.argv.slice(2));
const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
if (!token) {
  console.error("GITHUB_TOKEN or GH_TOKEN is required");
  process.exit(1);
}

const dates = await fetchStarDates(repo, token);
mkdirSync(dirname(output), { recursive: true });
writeFileSync(output, renderSvg(repo, dates), "utf8");
