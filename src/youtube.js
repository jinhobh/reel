// YouTube ads are served from the same hosts as the video, so network rules
// cannot block them. Strip ad metadata from player responses before the player
// reads it, with an auto-skip fallback for anything that slips through.
(() => {
  if (!/(^|\.)youtube\.com$/.test(location.hostname)) return;

  const KEYS = ["adPlacements", "adSlots", "playerAds", "adBreakHeartbeatParams"];
  const prune = (o) => {
    if (o && typeof o === "object") {
      for (const k of KEYS) if (k in o) delete o[k];
      if (o.playerResponse) prune(o.playerResponse);
    }
    return o;
  };

  const parse = JSON.parse;
  JSON.parse = function () {
    return prune(parse.apply(this, arguments));
  };

  const json = Response.prototype.json;
  Response.prototype.json = function () {
    return json.apply(this, arguments).then(prune);
  };

  let initial;
  Object.defineProperty(window, "ytInitialPlayerResponse", {
    configurable: true,
    get: () => initial,
    set: (v) => { initial = prune(v); },
  });

  setInterval(() => {
    const player = document.querySelector(".html5-video-player.ad-showing");
    if (!player) return;
    const v = player.querySelector("video");
    if (v && isFinite(v.duration)) { v.muted = true; v.currentTime = v.duration; }
    const skip = document.querySelector(".ytp-skip-ad-button, .ytp-ad-skip-button-modern, .ytp-ad-skip-button");
    if (skip) skip.click();
  }, 400);
})();
