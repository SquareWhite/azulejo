// Live reload for preview tiles (F9), shared by both shells.
//
// When a `preview.updated` event arrives for the file a tile already shows,
// the shell calls `azulejoPreview.swap(html)` with the event's full document
// instead of reloading the page, so the reader keeps their place. When the path
// changed, or the page isn't one of ours, `swap` returns false and the shell
// loads the document from scratch.
(function () {
  "use strict";

  function scrollRoot() {
    return document.scrollingElement || document.documentElement;
  }

  // The first element at least partly on screen, and how far it sits from the
  // top: a stable anchor when content above it grows or shrinks.
  function anchor(content) {
    var nodes = content.querySelectorAll("h1,h2,h3,h4,h5,h6,p,li,pre,table,tr,img,blockquote");
    for (var i = 0; i < nodes.length; i++) {
      var rect = nodes[i].getBoundingClientRect();
      if (rect.bottom > 0) {
        return { index: i, offset: rect.top };
      }
    }
    return null;
  }

  function swap(html) {
    var content = document.getElementById("azj-content");
    if (!content) {
      return false;
    }
    var next = new DOMParser().parseFromString(html, "text/html");
    var nextContent = next.getElementById("azj-content");
    if (!nextContent) {
      return false;
    }
    var root = scrollRoot();
    var atBottom = root.scrollTop + window.innerHeight >= root.scrollHeight - 2;
    var ratio = root.scrollHeight > window.innerHeight
      ? root.scrollTop / (root.scrollHeight - window.innerHeight)
      : 0;
    var mark = anchor(content);

    document.body.className = next.body.className;
    document.title = next.title;
    content.innerHTML = nextContent.innerHTML;

    if (atBottom) {
      // Following the end of a growing file (logs, a plan being written).
      root.scrollTop = root.scrollHeight;
    } else if (mark) {
      var nodes = content.querySelectorAll("h1,h2,h3,h4,h5,h6,p,li,pre,table,tr,img,blockquote");
      var target = nodes[Math.min(mark.index, nodes.length - 1)];
      if (target) {
        root.scrollTop += target.getBoundingClientRect().top - mark.offset;
      }
    } else {
      root.scrollTop = ratio * Math.max(0, root.scrollHeight - window.innerHeight);
    }
    return true;
  }

  window.azulejoPreview = { swap: swap };
})();
