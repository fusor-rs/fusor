import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const docs = new URL("../apps/docs/", import.meta.url);

// Read the authored headings and fences independently of the Rust renderer.
// These are expectations for the guide walkthroughs, not a Markdown renderer.
export async function readGuides() {
  const navigation = JSON.parse(await readFile(new URL("content/navigation.json", docs), "utf8"));
  return Promise.all(navigation.map(async entry => {
    const source = `content/${entry.slug || "index"}.md`;
    const markdown = await readFile(new URL(`public/${source}`, docs), "utf8");
    const blocks = [...markdown.matchAll(/^(`{3,}|~{3,})([^\n]*)\n([\s\S]*?)^\1[ \t]*(?:\n|$)/gm)];
    let outline = markdown;
    for (const block of blocks.toReversed()) {
      outline = outline.slice(0, block.index) + block[0].replace(/[^\n]/g, " ")
        + outline.slice(block.index + block[0].length);
    }
    const headings = [...outline.matchAll(/^## (.+) \{#([^}]+)\}$/gm)];
    const sections = await Promise.all(headings.map(async (heading, index) => {
      const end = headings[index + 1]?.index ?? markdown.length;
      const codes = await Promise.all(blocks.filter(block => block.index > heading.index && block.index < end)
        .map(readCode));
      return {
        id: heading[2], codes, code: codes[0]?.code,
        links: [...outline.slice(heading.index, end).matchAll(/\[[^\]]+\]\(([^)\s]+)\)/g)]
          .map(link => ({ href: link[1] })),
      };
    }));
    assert.equal(sections.length, [...outline.matchAll(/^## /gm)].length,
      `${source}: give guide sections explicit IDs for stable URLs`);
    return {
      ...entry, source, markdown, sections,
      title: markdown.match(/^# (.+)\n/)[1],
      anchors: [...outline.matchAll(/^#{2,6} .+ \{#([^}]+)\}$/gm)].map(heading => heading[1]),
    };
  }));
}

async function readCode(block) {
  const source = block[2].match(/(?:^| )source=(\S+)/)?.[1];
  return {
    code: source ? await readFile(new URL(source, docs), "utf8") : block[3],
    language: block[2].split(" title=")[1] ?? block[2],
  };
}
