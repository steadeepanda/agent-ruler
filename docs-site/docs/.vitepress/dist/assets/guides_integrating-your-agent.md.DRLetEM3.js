import { _ as _export_sfc, o as openBlock, c as createElementBlock, j as createBaseVNode, a as createTextVNode } from "./chunks/framework.UCBG75GM.js";
const __pageData = JSON.parse('{"title":"Integrating Your Agent","description":"","frontmatter":{},"headers":[],"relativePath":"guides/integrating-your-agent.md","filePath":"guides/integrating-your-agent.md"}');
const _sfc_main = { name: "guides/integrating-your-agent.md" };
function _sfc_render(_ctx, _cache, $props, $setup, $data, $options) {
  return openBlock(), createElementBlock("div", null, [..._cache[0] || (_cache[0] = [
    createBaseVNode("h1", {
      id: "integrating-your-agent",
      tabindex: "-1"
    }, [
      createTextVNode("Integrating Your Agent "),
      createBaseVNode("a", {
        class: "header-anchor",
        href: "#integrating-your-agent",
        "aria-label": 'Permalink to "Integrating Your Agent"'
      }, "​")
    ], -1),
    createBaseVNode("p", null, "This page moved into the Integrations section.", -1),
    createBaseVNode("p", null, "Use:", -1),
    createBaseVNode("ul", null, [
      createBaseVNode("li", null, [
        createBaseVNode("a", { href: "/help/integrations/openclaw-guide.html" }, "OpenClaw Guide"),
        createTextVNode(" for setup and operator workflow")
      ]),
      createBaseVNode("li", null, [
        createBaseVNode("a", { href: "/help/integrations/openclaw-api-reference.html" }, "OpenClaw API Reference"),
        createTextVNode(" for endpoint contracts")
      ])
    ], -1)
  ])]);
}
const integratingYourAgent = /* @__PURE__ */ _export_sfc(_sfc_main, [["render", _sfc_render]]);
export {
  __pageData,
  integratingYourAgent as default
};
