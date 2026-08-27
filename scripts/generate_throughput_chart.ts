#!/usr/bin/env bun

import { mkdirSync, readFileSync, writeFileSync } from "node:fs"
import { dirname, relative, resolve } from "node:path"
import { fileURLToPath } from "node:url"

type Chart_Kind = "slide" | "report"
type Row_Name = "OpenKache" | "PostgreSQL 17.10" | "MySQL 8.4.11"

interface Benchmark_Row {
  readonly name: Row_Name
  readonly throughput: number
}

interface Chart_Layout {
  readonly width: number
  readonly height: number
  readonly plot_left: number
  readonly plot_right: number
  readonly plot_top: number
  readonly plot_bottom: number
  readonly bar_height: number
  readonly bar_top_margin: number
  readonly bar_gap: number
  readonly title_x: number
  readonly title_y: number
  readonly title_size: number
  readonly label_size: number
  readonly value_size: number
  readonly tick_size: number
  readonly footer_size: number
  readonly label_gap: number
  readonly value_gap: number
  readonly label_baseline_offset: number
  readonly value_baseline_offset: number
  readonly tick_baseline_offset: number
  readonly footer_baseline_offset: number
}

interface Chart_Options {
  readonly benchmark_path: string
  readonly output_dir: string
}

const SCRIPT_DIRECTORY = dirname(fileURLToPath(import.meta.url))
const REPOSITORY_ROOT = resolve(SCRIPT_DIRECTORY, "..")
const DEFAULT_BENCHMARK_PATH = resolve(REPOSITORY_ROOT, "BENCHMARK.md")
const DEFAULT_OUTPUT_DIRECTORY = resolve(REPOSITORY_ROOT, "_local", "benchmark-charts")
const THROUGHPUT_SECTION_TITLE = "## Throughput (GET)"
const FOOTER = "100B random-access throughput benchmark"
const TITLE = "GET Throughput Benchmark"
const TICK_STEP = 20_000

const ROW_NAMES: readonly Row_Name[] = [
  "OpenKache",
  "PostgreSQL 17.10",
  "MySQL 8.4.11",
] as const

const BAR_COLORS: Record<Row_Name, string> = {
  OpenKache: "#16a6a4",
  "PostgreSQL 17.10": "#aab4c0",
  "MySQL 8.4.11": "#aab4c0",
}

const LAYOUTS: Record<Chart_Kind, Chart_Layout> = {
  slide: {
    width: 989.460951,
    height: 533.25,
    plot_left: 222.2475,
    plot_right: 897.6075,
    plot_top: 90.855,
    plot_bottom: 448.065,
    bar_height: 94.082498,
    bar_top_margin: 16.236818,
    bar_gap: 21.244935,
    title_x: 40.8075,
    title_y: 37.514062,
    title_size: 38,
    label_size: 22,
    value_size: 24,
    tick_size: 17,
    footer_size: 15,
    label_gap: 18,
    value_gap: 11.745392,
    label_baseline_offset: 8.358,
    value_baseline_offset: 6.234,
    tick_baseline_offset: 22.916,
    footer_baseline_offset: 12.244,
  },
  report: {
    width: 1122.078763,
    height: 272.808,
    plot_left: 290.3775,
    plot_right: 965.7375,
    plot_top: 77.436,
    plot_bottom: 189.756,
    bar_height: 33.28,
    bar_top_margin: 2.08,
    bar_gap: 4.16,
    title_x: 108.9375,
    title_y: 42.832969,
    title_size: 45,
    label_size: 30,
    value_size: 33,
    tick_size: 24,
    footer_size: 22.5,
    label_gap: 15,
    value_gap: 11.745392,
    label_baseline_offset: 11.397656,
    value_baseline_offset: 8.572266,
    tick_baseline_offset: 28.236375,
    footer_baseline_offset: 14.762727,
  },
} as const

function usage(): string {
  return [
    "Usage: bun scripts/generate_throughput_chart.ts [options]",
    "",
    "Options:",
    "  --benchmark <path>    Read throughput values from BENCHMARK.md.",
    "  --output-dir <path>  Write SVG charts to this directory.",
    "  --help                Show this message.",
    "",
    `Defaults: --benchmark ${relative(process.cwd(), DEFAULT_BENCHMARK_PATH)}`,
    `         --output-dir ${relative(process.cwd(), DEFAULT_OUTPUT_DIRECTORY)}`,
  ].join("\n")
}

function parse_options(raw_args: readonly string[]): Chart_Options {
  let benchmark_path = DEFAULT_BENCHMARK_PATH
  let output_dir = DEFAULT_OUTPUT_DIRECTORY

  for (let index = 0; index < raw_args.length; index += 1) {
    const argument = raw_args[index]
    if (argument === "--help" || argument === "-h") {
      console.log(usage())
      process.exit(0)
    }

    if (argument === "--benchmark" || argument === "--output-dir") {
      const value = raw_args[index + 1]
      if (value === undefined || value.startsWith("--")) {
        throw new Error(`${argument} requires a path.\n\n${usage()}`)
      }
      if (argument === "--benchmark") benchmark_path = resolve(process.cwd(), value)
      else output_dir = resolve(process.cwd(), value)
      index += 1
      continue
    }

    throw new Error(`Unknown option: ${argument}\n\n${usage()}`)
  }

  return { benchmark_path, output_dir }
}

function parse_throughput_rows(markdown: string, benchmark_path: string): readonly Benchmark_Row[] {
  const section_start = markdown.indexOf(THROUGHPUT_SECTION_TITLE)
  if (section_start < 0) {
    throw new Error(
      `Could not find "${THROUGHPUT_SECTION_TITLE}" in ${benchmark_path}. ` +
        "Keep the throughput table under that heading so the chart stays tied to the published measurements.",
    )
  }

  const next_section = markdown.indexOf("\n## ", section_start + THROUGHPUT_SECTION_TITLE.length)
  const section_end = next_section < 0 ? markdown.length : next_section
  const section = markdown.slice(section_start, section_end)
  const parsed_rows = new Map<string, number>()
  const row_pattern = /^\|\s*([^|]+?)\s*\|\s*([\d,]+)\s+ops\/s\s*\|/u

  for (const line of section.split("\n")) {
    const match = row_pattern.exec(line)
    if (match === null) continue

    const name = match[1]?.trim()
    const value_text = match[2]?.replaceAll(",", "")
    if (name === undefined || value_text === undefined) continue

    const throughput = Number(value_text)
    if (!Number.isSafeInteger(throughput) || throughput <= 0) {
      throw new Error(
        `Invalid throughput value for "${name}" in ${benchmark_path}: "${value_text}". ` +
          "Use a positive integer followed by ops/s in the throughput table.",
      )
    }
    if (parsed_rows.has(name)) {
      throw new Error(
        `Duplicate throughput row for "${name}" in ${benchmark_path}. ` +
          "Keep one canonical result per system so the chart is unambiguous.",
      )
    }
    parsed_rows.set(name, throughput)
  }

  const rows: Benchmark_Row[] = []
  for (const name of ROW_NAMES) {
    const throughput = parsed_rows.get(name)
    if (throughput === undefined) {
      throw new Error(
        `Missing throughput row for "${name}" in ${benchmark_path}. ` +
          "Add the measured ops/s value to the GET throughput table before generating the chart.",
      )
    }
    rows.push({ name, throughput })
  }
  return rows
}

function escape_xml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&apos;")
}

function format_number(value: number): string {
  return new Intl.NumberFormat("en-US").format(value)
}

function format_tick(value: number): string {
  if (value === 0) return "0"
  return `${value / 1_000}k`
}

function axis_maximum(rows: readonly Benchmark_Row[]): number {
  const maximum = Math.max(...rows.map((row) => row.throughput))
  return Math.max(TICK_STEP, Math.ceil(maximum / TICK_STEP) * TICK_STEP)
}

function row_bar_y(layout: Chart_Layout, row_index: number): number {
  return layout.plot_top + layout.bar_top_margin + row_index * (layout.bar_height + layout.bar_gap)
}

function render_chart(rows: readonly Benchmark_Row[], kind: Chart_Kind): string {
  const layout = LAYOUTS[kind]
  const maximum = axis_maximum(rows)
  const plot_width = layout.plot_right - layout.plot_left
  const description = rows
    .map((row) => `${row.name} achieved ${format_number(row.throughput)} ops/s`)
    .join(", ")
  const parts: string[] = [
    `<?xml version="1.0" encoding="utf-8"?>`,
    `<svg xmlns="http://www.w3.org/2000/svg" width="${layout.width}" height="${layout.height}" viewBox="0 0 ${layout.width} ${layout.height}" role="img" aria-labelledby="title description">`,
    `  <title id="title">${escape_xml(TITLE)}</title>`,
    `  <desc id="description">${escape_xml(description)}.</desc>`,
    "  <style>",
    "    .background { fill: #ffffff; }",
    "    .title { fill: #101828; font: 700 var(--title-size) 'DejaVu Sans', Arial, sans-serif; }",
    "    .label { fill: #101828; font: 400 var(--label-size) 'DejaVu Sans', Arial, sans-serif; }",
    "    .label-openkache { font-weight: 700; }",
    "    .value { fill: #101828; font: 700 var(--value-size) 'DejaVu Sans', Arial, sans-serif; }",
    "    .tick { fill: #667085; font: 400 var(--tick-size) 'DejaVu Sans', Arial, sans-serif; }",
    "    .footer { fill: #667085; font: 400 var(--footer-size) 'DejaVu Sans', Arial, sans-serif; }",
    "    .grid { stroke: #e4e7ec; stroke-width: 1.15; }",
    "  </style>",
    `  <g style="--title-size:${layout.title_size}px;--label-size:${layout.label_size}px;--value-size:${layout.value_size}px;--tick-size:${layout.tick_size}px;--footer-size:${layout.footer_size}px">`,
    `    <rect class="background" width="${layout.width}" height="${layout.height}" />`,
    `    <text class="title" x="${layout.title_x}" y="${layout.title_y}">${escape_xml(TITLE)}</text>`,
  ]

  for (let tick = 0; tick <= maximum; tick += TICK_STEP) {
    const x = layout.plot_left + (tick / maximum) * plot_width
    parts.push(
      `    <line class="grid" x1="${x}" y1="${layout.plot_top}" x2="${x}" y2="${layout.plot_bottom}" />`,
      `    <text class="tick" x="${x}" y="${layout.plot_bottom + layout.tick_baseline_offset}" text-anchor="middle">${escape_xml(format_tick(tick))}</text>`,
    )
  }

  for (const [row_index, row] of rows.entries()) {
    const bar_y = row_bar_y(layout, row_index)
    const bar_width = (row.throughput / maximum) * plot_width
    const bar_end = layout.plot_left + bar_width
    const label_y = bar_y + layout.bar_height / 2 + layout.label_baseline_offset
    const value_y = bar_y + layout.bar_height / 2 + layout.value_baseline_offset
    const label_class = row.name === "OpenKache" ? "label label-openkache" : "label"
    parts.push(
      `    <text class="${label_class}" x="${layout.plot_left - layout.label_gap}" y="${label_y}" text-anchor="end">${escape_xml(row.name)}</text>`,
      `    <rect x="${layout.plot_left}" y="${bar_y}" width="${bar_width}" height="${layout.bar_height}" fill="${BAR_COLORS[row.name]}" />`,
      `    <text class="value" x="${bar_end + layout.value_gap}" y="${value_y}">${escape_xml(`${format_number(row.throughput)} ops/s`)}</text>`,
    )
  }

  parts.push(
    `    <text class="footer" x="${layout.width / 2}" y="${layout.height - layout.footer_baseline_offset}" text-anchor="middle">${escape_xml(FOOTER)}</text>`,
    "  </g>",
    "</svg>",
    "",
  )
  return parts.join("\n")
}

function write_charts(rows: readonly Benchmark_Row[], output_dir: string): void {
  mkdirSync(output_dir, { recursive: true })
  const charts: readonly [Chart_Kind, string][] = [
    ["slide", "throughput-get-horizontal.svg"],
    ["report", "throughput-get-horizontal-report.svg"],
  ]

  for (const [kind, filename] of charts) {
    const output_path = resolve(output_dir, filename)
    writeFileSync(output_path, render_chart(rows, kind), "utf8")
    console.log(`Wrote ${output_path}`)
  }
}

function main(): void {
  try {
    const options = parse_options(Bun.argv.slice(2))
    const markdown = readFileSync(options.benchmark_path, "utf8")
    const rows = parse_throughput_rows(markdown, options.benchmark_path)
    write_charts(rows, options.output_dir)
  } catch (error: unknown) {
    const message = error instanceof Error ? error.message : String(error)
    console.error(`Throughput chart generation failed.\n${message}`)
    process.exitCode = 1
  }
}

if (import.meta.main) main()
