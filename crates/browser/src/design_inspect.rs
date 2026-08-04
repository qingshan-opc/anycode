//! In-page Design inspect helpers (CDP Runtime.evaluate).
//!
//! Used when the Workbench embeds CEF: React overlays cannot sit above the
//! native Chromium view, so pick/hover must run inside the page.

use crate::error::{BrowserError, BrowserResult};
use crate::types::{BrowserDesignInspectState, BrowserHitTestResult};
use chromiumoxide::Page;
use serde::Deserialize;

/// Install (or no-op if already present) the page-side inspect controller.
pub const ENABLE_SCRIPT: &str = r##"(() => {
  if (window.__anycodeDesignInspect && window.__anycodeDesignInspect.enabled) {
    return 'already';
  }
  const prev = window.__anycodeDesignInspect;
  if (prev && typeof prev.disable === 'function') {
    try { prev.disable(); } catch (_) {}
  }

  const state = {
    enabled: true,
    hoverEl: null,
    overlay: null,
    tagEl: null,
    lastPick: null,
    hoverHit: null,
  };

  function ensureUi() {
    if (!state.overlay) {
      const d = document.createElement('div');
      d.setAttribute('data-anycode-design', 'overlay');
      Object.assign(d.style, {
        position: 'fixed',
        pointerEvents: 'none',
        zIndex: '2147483646',
        border: '2px solid #7868ff',
        background: 'rgba(120, 104, 255, 0.14)',
        display: 'none',
        boxSizing: 'border-box',
        borderRadius: '2px',
      });
      document.documentElement.appendChild(d);
      state.overlay = d;
    }
    if (!state.tagEl) {
      const t = document.createElement('div');
      t.setAttribute('data-anycode-design', 'tag');
      Object.assign(t.style, {
        position: 'fixed',
        pointerEvents: 'none',
        zIndex: '2147483647',
        background: '#7868ff',
        color: '#fff',
        font: '11px/1.2 ui-sans-serif, system-ui, sans-serif',
        padding: '2px 6px',
        borderRadius: '4px',
        display: 'none',
        maxWidth: '240px',
        whiteSpace: 'nowrap',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
      });
      document.documentElement.appendChild(t);
      state.tagEl = t;
    }
  }

  function escId(value) {
    if (window.CSS && CSS.escape) return CSS.escape(value);
    return String(value).replace(/([^\w-])/g, '\\$1');
  }

  function cssPath(node) {
    if (!(node instanceof Element)) return '';
    if (node.id) return '#' + escId(node.id);
    const parts = [];
    let cur = node;
    while (cur && cur.nodeType === 1 && parts.length < 6) {
      let part = cur.tagName.toLowerCase();
      if (cur.id) {
        parts.unshift('#' + escId(cur.id));
        break;
      }
      const parent = cur.parentElement;
      if (parent) {
        const siblings = Array.from(parent.children).filter((c) => c.tagName === cur.tagName);
        if (siblings.length > 1) {
          const idx = siblings.indexOf(cur) + 1;
          part += ':nth-of-type(' + idx + ')';
        }
      }
      parts.unshift(part);
      cur = parent;
      if (cur && cur.tagName && cur.tagName.toLowerCase() === 'body') {
        parts.unshift('body');
        break;
      }
    }
    return parts.join(' > ');
  }

  function xpathOf(node) {
    if (!(node instanceof Element)) return null;
    if (node.id) return '//*[@id="' + String(node.id).replace(/"/g, '\\"') + '"]';
    const parts = [];
    let cur = node;
    while (cur && cur.nodeType === 1 && parts.length < 8) {
      let ix = 1;
      let sib = cur.previousElementSibling;
      while (sib) {
        if (sib.tagName === cur.tagName) ix++;
        sib = sib.previousElementSibling;
      }
      parts.unshift(cur.tagName.toLowerCase() + '[' + ix + ']');
      cur = cur.parentElement;
      if (cur && cur.tagName && cur.tagName.toLowerCase() === 'html') break;
    }
    return '/' + parts.join('/');
  }

  function describe(el, x, y) {
    if (!(el instanceof Element)) return null;
    const tag = el.tagName.toLowerCase();
    const id = el.id || null;
    const classes = el.classList ? Array.from(el.classList).slice(0, 8) : [];
    let text = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ');
    if (text.length > 80) text = text.slice(0, 79) + '…';
    return {
      x: x,
      y: y,
      tag: tag,
      id: id,
      classes: classes,
      text: text || null,
      css_selector: cssPath(el),
      xpath: xpathOf(el),
    };
  }

  function highlight(el) {
    ensureUi();
    if (!(el instanceof Element) || el === document.documentElement || el === document.body) {
      state.overlay.style.display = 'none';
      state.tagEl.style.display = 'none';
      state.hoverEl = null;
      state.hoverHit = null;
      return;
    }
    const r = el.getBoundingClientRect();
    Object.assign(state.overlay.style, {
      display: 'block',
      left: Math.max(0, r.left) + 'px',
      top: Math.max(0, r.top) + 'px',
      width: Math.max(0, r.width) + 'px',
      height: Math.max(0, r.height) + 'px',
    });
    const label = (el.tagName.toLowerCase()) + (el.id ? '#' + el.id : '');
    state.tagEl.textContent = label;
    Object.assign(state.tagEl.style, {
      display: 'block',
      left: Math.min(window.innerWidth - 8, Math.max(0, r.left)) + 'px',
      top: Math.max(0, r.top - 18) + 'px',
    });
    state.hoverEl = el;
  }

  function onMove(e) {
    if (!state.enabled) return;
    const el = document.elementFromPoint(e.clientX, e.clientY);
    if (el && el.getAttribute && el.getAttribute('data-anycode-design')) return;
    highlight(el);
    state.hoverHit = describe(el, e.clientX, e.clientY);
  }

  function onPick(e) {
    if (!state.enabled) return;
    const el = document.elementFromPoint(e.clientX, e.clientY);
    if (el && el.getAttribute && el.getAttribute('data-anycode-design')) return;
    e.preventDefault();
    e.stopPropagation();
    if (typeof e.stopImmediatePropagation === 'function') e.stopImmediatePropagation();
    highlight(el);
    const hit = describe(el, e.clientX, e.clientY);
    state.hoverHit = hit;
    state.lastPick = hit;
  }

  function onKey(e) {
    if (!state.enabled) return;
    if (e.key === 'Escape') {
      state.lastPick = null;
      highlight(null);
    }
  }

  document.addEventListener('mousemove', onMove, true);
  document.addEventListener('click', onPick, true);
  document.addEventListener('pointerdown', onPick, true);
  document.addEventListener('keydown', onKey, true);
  document.documentElement.style.cursor = 'crosshair';

  window.__anycodeDesignInspect = {
    enabled: true,
    disable() {
      state.enabled = false;
      document.removeEventListener('mousemove', onMove, true);
      document.removeEventListener('click', onPick, true);
      document.removeEventListener('pointerdown', onPick, true);
      document.removeEventListener('keydown', onKey, true);
      document.documentElement.style.cursor = '';
      if (state.overlay && state.overlay.parentNode) state.overlay.parentNode.removeChild(state.overlay);
      if (state.tagEl && state.tagEl.parentNode) state.tagEl.parentNode.removeChild(state.tagEl);
      state.overlay = null;
      state.tagEl = null;
      state.hoverEl = null;
      state.hoverHit = null;
      state.lastPick = null;
      this.enabled = false;
    },
    takePick() {
      const p = state.lastPick;
      state.lastPick = null;
      return p;
    },
    getHover() {
      return state.hoverHit;
    },
  };
  return 'ok';
})()"##;

pub const DISABLE_SCRIPT: &str = r##"(() => {
  if (window.__anycodeDesignInspect && typeof window.__anycodeDesignInspect.disable === 'function') {
    window.__anycodeDesignInspect.disable();
  }
  try { delete window.__anycodeDesignInspect; } catch (_) { window.__anycodeDesignInspect = undefined; }
  return 'ok';
})()"##;

pub const POLL_SCRIPT: &str = r##"(() => {
  const api = window.__anycodeDesignInspect;
  if (!api || !api.enabled) {
    return { enabled: false, hover: null, pick: null };
  }
  return {
    enabled: true,
    hover: typeof api.getHover === 'function' ? api.getHover() : null,
    pick: typeof api.takePick === 'function' ? api.takePick() : null,
  };
})()"##;

#[derive(Debug, Deserialize)]
struct JsHit {
    x: f64,
    y: f64,
    tag: String,
    id: Option<String>,
    #[serde(default)]
    classes: Vec<String>,
    text: Option<String>,
    css_selector: String,
    xpath: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JsPoll {
    enabled: bool,
    hover: Option<JsHit>,
    pick: Option<JsHit>,
}

fn map_hit(h: JsHit) -> BrowserHitTestResult {
    BrowserHitTestResult {
        x: h.x,
        y: h.y,
        tag: h.tag,
        id: h.id.filter(|s| !s.is_empty()),
        classes: h.classes,
        text: h.text.filter(|s| !s.is_empty()),
        css_selector: h.css_selector,
        xpath: h.xpath.filter(|s| !s.is_empty()),
    }
}

pub async fn enable_on_page(page: &Page) -> BrowserResult<()> {
    page.evaluate(ENABLE_SCRIPT)
        .await
        .map_err(|e| BrowserError::Other(e.into()))?;
    Ok(())
}

pub async fn disable_on_page(page: &Page) -> BrowserResult<()> {
    page.evaluate(DISABLE_SCRIPT)
        .await
        .map_err(|e| BrowserError::Other(e.into()))?;
    Ok(())
}

pub async fn poll_on_page(page: &Page) -> BrowserResult<BrowserDesignInspectState> {
    let raw: JsPoll = page
        .evaluate(POLL_SCRIPT)
        .await
        .map_err(|e| BrowserError::Other(e.into()))?
        .into_value()
        .map_err(|e| BrowserError::Other(e.into()))?;
    Ok(BrowserDesignInspectState {
        enabled: raw.enabled,
        hover: raw.hover.map(map_hit),
        pick: raw.pick.map(map_hit),
    })
}
