import * as s from process.env.PLANE_EDITOR_SERVER ?? "../../../plane/packages/editor/dist/server.js";
import fs from "fs";
import extra from "./extra.mjs";
const cases = {
  basic: '<p>Hello <strong>world</strong> and <em>it</em> <u>u</u> <s>s</s> <code>c</code> <a href="https://x.dev">link</a></p>',
  headings: '<h1>One</h1><h2>Two</h2><h3>Three</h3><p></p>',
  lists: '<ul><li><p>a</p></li><li><p>b</p><ul><li><p>c</p></li></ul></li></ul><ol><li><p>1</p></li></ol>',
  tasks: '<ul data-type="taskList"><li data-type="taskItem" data-checked="true"><p>done</p></li><li data-type="taskItem" data-checked="false"><p>todo</p></li></ul>',
  quote_code: '<blockquote><p>q</p></blockquote><pre><code class="language-rust">fn main() {}\nx &lt; y</code></pre><hr>',
  mention: '<p>hi <mention-component id="m1" entity_identifier="11111111-1111-1111-1111-111111111111" entity_name="user_mention"></mention-component></p>',
  image: '<image-component src="22222222-2222-2222-2222-222222222222" width="35%" height="auto" aspectratio="1.5" alignment="center" id="img1"></image-component>',
  callout: '<div data-block-type="callout-component" data-icon-color="#6d7b8a" data-icon-name="FileText" data-emoji-unicode="128512" data-emoji-url="https://cdn.jsdelivr.net/npm/emoji-datasource-apple/img/apple/64/1f600.png" data-logo-in-use="emoji" data-background="gray"><p>note</p></div>',
  table: '<table><tbody><tr><th><p>h</p></th><th><p>h2</p></th></tr><tr><td><p>c</p></td><td><p>d</p></td></tr></tbody></table>',
  color: '<p><span style="color: var(--editor-colors-red-text)" data-text-color="red">red</span> <span data-background-color="blue" style="background-color: var(--editor-colors-blue-background)">bg</span></p>',
  align: '<p style="text-align: center">mid</p><h2 style="text-align: right">r</h2>',
  breaks: '<p>a<br>b</p>',
  escape: '<p>&lt;script&gt;alert(1)&lt;/script&gt; &amp; "q"</p>',
  ...extra,
};
const out = {};
for (const [k, html] of Object.entries(cases)) {
  const bin = s.getBinaryDataFromDocumentEditorHTMLString(html, "Title " + k);
  const f = s.getAllDocumentFormatsFromDocumentEditorBinaryData(bin, true);
  out[k] = { input: html, binary: Buffer.from(bin).toString("base64"), html: f.contentHTML, json: f.contentJSON };
}
fs.writeFileSync(process.argv[2], JSON.stringify(out, null, 1));
for (const [k, v] of Object.entries(out)) console.log(k, "=>", v.html);
