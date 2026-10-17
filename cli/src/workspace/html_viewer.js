// video-perception Observation viewer v5 — Chart.js multi-resolution time-series.
//
// SECURITY NOTE: Offline-only viewer for locally generated sensor data.
// All dynamic content escaped via esc(). No external/untrusted input.

// ── State ──
var totalMs = 0, viewStart = 0, viewEnd = 0;
var selectedMs = -1, currentLayer = 'overview';
var layerHistory = [];
var currentMode = 'analysis';
var charts = {};  // chartId -> Chart instance

// Pre-computed
var buckets5s = {}, buckets1s = {};
var trackKeys = ['y', 'sa', 'range', 'diff'];

// ── Helpers ──
function msToTime(ms) { var s = ms/1000, m = Math.floor(s/60), sec = (s%60).toFixed(1); return m+':'+('000'+sec).slice(-4); }
// Escapes for both text and attribute context: file names come from untrusted
// media, so quotes must not break out of src="..." or data-*="..." values.
function esc(s) { var d = document.createElement('div'); d.textContent = s == null ? '' : String(s); return d.innerHTML.replace(/"/g, '&quot;').replace(/'/g, '&#39;'); }
function fmtSize(b) { return b>1048576?(b/1048576).toFixed(1)+'MB':b>1024?(b/1024).toFixed(0)+'KB':b+'B'; }
function tagHtml(t) { return '<span class="tag '+esc(t)+'">'+esc(t)+'</span>'; }
function resolvePath(o,p) { var a=p.split('.'); for(var i=0;i<a.length;i++){if(o==null)return undefined;o=o[a[i]];} return o; }
function sceneAt(ms) { for(var i=0;i<SCENES.length;i++) if(ms>=SCENES[i].start_ms&&ms<SCENES[i].end_ms) return i; return -1; }
function trackFrameAt(ms) { if(!TRACK||!TRACK.length||totalMs<=0)return null; var fi=Math.round((ms/totalMs)*(TRACK.length-1)); return TRACK[Math.max(0,Math.min(TRACK.length-1,fi))]; }
function setMode(mode) { currentMode=mode; document.getElementById('modeAnalysis').style.display=mode==='analysis'?'':'none'; document.getElementById('modeFiles').style.display=mode==='files'?'':'none'; document.getElementById('btnAnalysis').classList.toggle('active',mode==='analysis'); document.getElementById('btnFiles').classList.toggle('active',mode==='files'); if(mode==='files')renderFinderTree(); }

// ════════════════════════════════════════════════════════════════
// BUCKETING + DERIVED FIELDS
// ════════════════════════════════════════════════════════════════

function precompute() {
  if (!TRACK || !TRACK.length) return;
  // Derive fields FIRST, before bucketing
  for (var i = 0; i < TRACK.length; i++) {
    TRACK[i].range = TRACK[i].yx - TRACK[i].yn;
    TRACK[i].diff = i > 0 ? Math.abs(TRACK[i].y - TRACK[i-1].y) : 0;
  }
  // Now bucket all keys including diff
  for (var ki = 0; ki < trackKeys.length; ki++) {
    buckets5s[trackKeys[ki]] = buildBuckets(trackKeys[ki], 5000);
    buckets1s[trackKeys[ki]] = buildBuckets(trackKeys[ki], 1000);
  }
}

function buildBuckets(key, windowMs) {
  var result = [], bi = 0;
  while (bi * windowMs < totalMs) {
    var tS = bi*windowMs, tE = Math.min(tS+windowMs, totalMs), vals = [];
    for (var i = 0; i < TRACK.length; i++) if (TRACK[i].t >= tS && TRACK[i].t < tE) vals.push(TRACK[i][key]);
    if (vals.length) {
      var sum=0,mn=Infinity,mx=-Infinity;
      for(var j=0;j<vals.length;j++){sum+=vals[j];if(vals[j]<mn)mn=vals[j];if(vals[j]>mx)mx=vals[j];}
      result.push({t:(tS+tE)/2, tStart:tS, tEnd:tE, mean:sum/vals.length, min:mn, max:mx});
    }
    bi++;
  }
  return result;
}

function getDataForLayer(key, layer, vS, vE) {
  var data;
  if (layer === 'frame') {
    data = []; for (var i=0;i<TRACK.length;i++) if(TRACK[i].t>=vS&&TRACK[i].t<=vE) data.push({t:TRACK[i].t, mean:TRACK[i][key], min:TRACK[i][key], max:TRACK[i][key]});
  } else if (layer === '1s') {
    data = []; var bk=buckets1s[key]; if(bk) for(var i=0;i<bk.length;i++) if(bk[i].tEnd>vS&&bk[i].tStart<vE) data.push(bk[i]);
  } else {
    // overview and 5s both use 5s buckets
    data = []; var bk=buckets5s[key]; if(bk) for(var i=0;i<bk.length;i++) if(bk[i].tEnd>vS&&bk[i].tStart<vE) data.push(bk[i]);
  }
  return data;
}

// ════════════════════════════════════════════════════════════════
// CHART.JS RENDERING
// ════════════════════════════════════════════════════════════════

var chartConfigs = [
  {id:'Luma',  key:'y',     color:'rgba(212,212,212,1)',  fill:'rgba(212,212,212,0.15)', label:'Luminance'},
  {id:'Sat',   key:'sa',    color:'rgba(99,102,241,1)',   fill:'rgba(99,102,241,0.15)',  label:'Saturation'},
  {id:'Range', key:'range', color:'rgba(245,158,11,1)',   fill:'rgba(245,158,11,0.15)',  label:'Contrast'},
  {id:'Diff',  key:'diff',  color:'rgba(239,68,68,1)',    fill:'rgba(239,68,68,0.15)',   label:'Frame Diff'},
];

var sceneChartConfigs = [
  {id:'Motion', valueFn:function(s){return s.track?s.track.motion_intensity:null;}, color:'rgba(249,115,22,1)', fill:'rgba(249,115,22,0.2)', label:'Motion'},
  {id:'Sharp',  valueFn:function(s){return s.blur?s.blur.laplacian_variance:null;}, color:'rgba(168,85,247,1)', fill:'rgba(168,85,247,0.2)', label:'Sharpness'},
  {id:'Static', valueFn:function(s){return s.signalstats?s.signalstats.static_frame_ratio:null;}, color:'rgba(220,38,38,1)', fill:'rgba(220,38,38,0.2)', label:'Static Ratio'},
  {id:'Audio',  valueFn:function(s){return s.audio?s.audio.input_i:null;}, color:'rgba(16,185,129,1)', fill:'rgba(16,185,129,0.2)', label:'Audio LUFS'},
];

function renderTimeline() {
  totalMs = TRACK&&TRACK.length ? TRACK[TRACK.length-1].t : (SCENES.length?SCENES[SCENES.length-1].end_ms:0);
  if (totalMs <= 0) return;
  viewStart = 0; viewEnd = totalMs; currentLayer = 'overview';
  precompute();
  renderAllCharts();
  setupInteractions();
  updateLayerUI();
}

function renderAllCharts() {
  var vS = viewStart, vE = viewEnd;

  // Track-based lanes (multi-resolution via Chart.js)
  for (var i = 0; i < chartConfigs.length; i++) {
    var cfg = chartConfigs[i];
    var data = TRACK && TRACK.length ? getDataForLayer(cfg.key, currentLayer, vS, vE) : [];
    renderChart(cfg, data, currentLayer);
  }

  // Scene-based lanes
  for (var i = 0; i < sceneChartConfigs.length; i++) {
    var cfg = sceneChartConfigs[i];
    var data = [];
    for (var si = 0; si < SCENES.length; si++) {
      var s = SCENES[si], val = cfg.valueFn(s);
      if (val == null) continue;
      var mid = (s.start_ms + s.end_ms) / 2;
      if (mid < vS || mid > vE) continue;
      data.push({t: mid, mean: val, min: val, max: val});
    }
    renderChart(cfg, data, 'scene');
  }

  // Color + Scenes lanes (SVG, kept as-is)
  renderColorLane(vS, vE, 80);
  renderScenesLane(vS, vE, 80);

  // Keyframe observation strip
  renderKeyframeStrip(vS, vE);
}

function renderChart(cfg, data, layer) {
  var canvasId = 'chart' + cfg.id;
  var canvas = document.getElementById(canvasId);
  if (!canvas) return;

  // Destroy existing chart
  if (charts[canvasId]) { charts[canvasId].destroy(); delete charts[canvasId]; }

  if (!data.length) return;

  var labels = data.map(function(d) { return (d.t / 1000).toFixed(1); });
  var means = data.map(function(d) { return d.mean; });
  var mins = data.map(function(d) { return d.min; });
  var maxs = data.map(function(d) { return d.max; });

  var datasets = [];

  // Min/max band for bucket layers
  if (layer === '5s' || layer === '1s' || layer === 'overview') {
    var hasBand = false;
    for (var i = 0; i < data.length; i++) { if (data[i].min !== data[i].max) { hasBand = true; break; } }
    if (hasBand) {
      datasets.push({
        label: 'Max',
        data: maxs,
        borderColor: 'transparent',
        backgroundColor: 'transparent',
        pointRadius: 0,
        fill: false,
      });
      datasets.push({
        label: 'Min',
        data: mins,
        borderColor: 'transparent',
        backgroundColor: cfg.fill,
        pointRadius: 0,
        fill: '-1',  // fill to previous dataset (max)
      });
    }
  }

  // Mean line
  datasets.push({
    label: cfg.label,
    data: means,
    borderColor: cfg.color,
    backgroundColor: cfg.fill,
    borderWidth: 1.5,
    pointRadius: layer === 'frame' ? 2.5 : (layer === 'scene' ? 3 : 0),
    pointBackgroundColor: cfg.color,
    fill: datasets.length === 0,  // fill to bottom if no band
    tension: layer === 'frame' ? 0 : 0.3,
  });

  charts[canvasId] = new Chart(canvas, {
    type: 'line',
    data: { labels: labels, datasets: datasets },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      animation: false,
      interaction: { mode: 'index', intersect: false },
      plugins: {
        legend: { display: false },
        tooltip: {
          enabled: true,
          backgroundColor: '#1a1a1a',
          titleColor: '#888',
          bodyColor: '#d4d4d4',
          borderColor: '#333',
          borderWidth: 1,
          titleFont: { family: 'monospace', size: 10 },
          bodyFont: { family: 'monospace', size: 11 },
          callbacks: {
            title: function(items) { return items[0] ? items[0].label + 's' : ''; },
            label: function(item) {
              if (item.dataset.label === 'Max' || item.dataset.label === 'Min') return null;
              return item.dataset.label + ': ' + (typeof item.raw === 'number' ? item.raw.toFixed(1) : item.raw);
            }
          }
        }
      },
      scales: {
        x: {
          display: true,
          grid: { color: 'rgba(60,60,60,0.3)', drawTicks: false },
          ticks: { color: '#555', font: { family: 'monospace', size: 9 }, maxTicksLimit: 10, callback: function(v,i) { return labels[i] ? labels[i]+'s' : ''; } },
        },
        y: {
          display: true,
          grid: { color: 'rgba(60,60,60,0.3)', drawTicks: false },
          ticks: { color: '#555', font: { family: 'monospace', size: 9 }, maxTicksLimit: 4 },
        }
      },
      onClick: function(evt) {
        var rect = canvas.getBoundingClientRect();
        var x = evt.native.clientX - rect.left;
        var pct = x / rect.width;
        var ms = viewStart + pct * (viewEnd - viewStart);
        selectedMs = ms;
        renderDetail(ms);
        // Sync video
        var vid = document.getElementById('videoPlayer');
        if (vid) { try { vid.currentTime = ms/1000; } catch(e) {} }
      }
    }
  });

  // Update range label
  var rangeEl = document.getElementById('range' + cfg.id);
  if (rangeEl && means.length) {
    var lo = Math.min.apply(null, mins), hi = Math.max.apply(null, maxs);
    rangeEl.textContent = (lo < 1 ? lo.toFixed(2) : Math.round(lo)) + '\u2013' + (hi < 1 ? hi.toFixed(2) : Math.round(hi));
  }
}

// ── Color + Scenes lanes (SVG — no Chart.js equivalent) ──

var W = 1000, LANE_H = 80;

function renderColorLane(vS, vE, H) {
  var el = document.querySelector('#laneColor .tl-chart svg'); if (!el) return;
  function xOf(t) { return ((t-vS)/(vE-vS))*W; }
  var svg = '';
  if (typeof COLOR_TRACK !== 'undefined' && COLOR_TRACK && COLOR_TRACK.length > 1) {
    for (var i=0;i<COLOR_TRACK.length;i++) { var ct=COLOR_TRACK[i]; if(ct.t>vE)break; var nt=(i+1<COLOR_TRACK.length)?COLOR_TRACK[i+1].t:totalMs; if(nt<vS)continue; var x1=Math.max(0,xOf(ct.t)),x2=Math.min(W,xOf(nt)); var colors=ct.colors||['#333']; var ch=H/colors.length; for(var c=0;c<colors.length;c++) svg+='<rect x="'+x1.toFixed(1)+'" y="'+(c*ch)+'" width="'+(x2-x1).toFixed(1)+'" height="'+ch+'" fill="'+esc(colors[c])+'"/>'; }
  } else {
    for (var i=0;i<SCENES.length;i++) { var s=SCENES[i]; if(s.end_ms<vS||s.start_ms>vE)continue; var x1=Math.max(0,xOf(s.start_ms)),x2=Math.min(W,xOf(s.end_ms)); var colors=(s.stats&&s.stats.dominant_colors)||['#333']; var ch=H/colors.length; for(var c=0;c<colors.length;c++) svg+='<rect x="'+x1.toFixed(1)+'" y="'+(c*ch)+'" width="'+(x2-x1).toFixed(1)+'" height="'+ch+'" fill="'+esc(colors[c])+'"/>'; }
  }
  el.setAttribute('viewBox','0 0 '+W+' '+H); el.setAttribute('preserveAspectRatio','none'); el.innerHTML = svg;
}

function renderScenesLane(vS, vE, H) {
  var el = document.querySelector('#laneScenes .tl-chart svg'); if (!el) return;
  function xOf(t) { return ((t-vS)/(vE-vS))*W; }
  var palette = ['#2a3a5c','#3a2a5c','#2a5c3a','#5c3a2a','#5c5c2a','#2a5c5c','#5c2a5c','#4a3a2a'];
  var svg = '';
  for (var i=0;i<SCENES.length;i++) { var s=SCENES[i]; if(s.end_ms<vS||s.start_ms>vE)continue; var x1=Math.max(0,xOf(s.start_ms)),x2=Math.min(W,xOf(s.end_ms)); svg+='<rect class="scene-block" x="'+x1.toFixed(1)+'" y="'+(H*0.15)+'" width="'+(x2-x1).toFixed(1)+'" height="'+(H*0.7)+'" fill="'+palette[i%palette.length]+'" rx="2"/>'; if((x2-x1)>25) svg+='<text x="'+((x1+x2)/2)+'" y="'+(H*0.6)+'" fill="#888" font-family="monospace" font-size="10" text-anchor="middle">S'+i+'</text>'; }
  el.setAttribute('viewBox','0 0 '+W+' '+H); el.setAttribute('preserveAspectRatio','none'); el.innerHTML = svg;
}

// ════════════════════════════════════════════════════════════════
// LAYER NAVIGATION
// ════════════════════════════════════════════════════════════════

function setLayer(layer, start, end) {
  if (currentLayer !== layer) layerHistory.push({layer:currentLayer,start:viewStart,end:viewEnd});
  currentLayer = layer;
  if (start!=null&&end!=null) {viewStart=start;viewEnd=end;}
  else if (layer==='overview') {viewStart=0;viewEnd=totalMs;}
  renderAllCharts();
  updateLayerUI();
}

function layerBack() {
  if(!layerHistory.length){setLayer('overview');return;}
  var prev=layerHistory.pop();currentLayer=prev.layer;viewStart=prev.start;viewEnd=prev.end;
  renderAllCharts();updateLayerUI();
}

function drillTo(layer, centerMs) {
  var windowMs = layer==='5s'?15000:layer==='1s'?5000:1500;
  var s=centerMs-windowMs/2, e=centerMs+windowMs/2;
  if(s<0){e-=s;s=0;} if(e>totalMs){s-=(e-totalMs);e=totalMs;} s=Math.max(0,s);
  setLayer(layer,s,e);
}

function updateLayerUI() {
  ['btnL0','btnL1','btnL2','btnL3'].forEach(function(id){var el=document.getElementById(id);if(el)el.classList.remove('active');});
  var aid=currentLayer==='overview'?'btnL0':currentLayer==='5s'?'btnL1':currentLayer==='1s'?'btnL2':'btnL3';
  var ael=document.getElementById(aid);if(ael)ael.classList.add('active');
  var bb=document.getElementById('btnBack');if(bb)bb.style.display=currentLayer==='overview'?'none':'';
  var info=document.getElementById('zoomInfo');
  if(info) info.textContent = currentLayer==='overview' ? 'Full video \u00b7 '+(totalMs/1000).toFixed(1)+'s' : msToTime(viewStart)+' \u2013 '+msToTime(viewEnd)+' \u00b7 '+((viewEnd-viewStart)/1000).toFixed(1)+'s \u00b7 '+currentLayer;
}

// ── Interactions ──
function setupInteractions() {
  var tl = document.getElementById('timeline'); if(!tl)return;
  tl.addEventListener('mousemove', function(e) { var ms=mouseToMs(e); if(ms<0)return; updateReadout(ms); });
  tl.addEventListener('dblclick', function(e) {
    e.preventDefault(); var ms=mouseToMs(e); if(ms<0)return;
    var next=currentLayer==='overview'?'5s':currentLayer==='5s'?'1s':currentLayer==='1s'?'frame':null;
    if(next) drillTo(next,ms);
  });
  tl.addEventListener('wheel', function(e) {
    // Only capture horizontal scroll for timeline panning; let vertical scroll through for page
    if (Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return;
    e.preventDefault(); var range=viewEnd-viewStart;
    var panMs=(e.deltaX/500)*range;
    var ns=viewStart+panMs,ne=viewEnd+panMs;
    if(ns<0){ne-=ns;ns=0;} if(ne>totalMs){ns-=(ne-totalMs);ne=totalMs;} ns=Math.max(0,ns);
    viewStart=ns;viewEnd=ne; renderAllCharts();
  }, {passive:false});
  document.addEventListener('keydown', function(e) {
    if(currentMode!=='analysis')return; var range=viewEnd-viewStart;
    if(e.key==='ArrowRight'){e.preventDefault();panBy(range*0.2);}
    else if(e.key==='ArrowLeft'){e.preventDefault();panBy(-range*0.2);}
    else if(e.key==='ArrowDown'){e.preventDefault();var n=currentLayer==='overview'?'5s':currentLayer==='5s'?'1s':currentLayer==='1s'?'frame':null;if(n)drillTo(n,selectedMs>=0?selectedMs:(viewStart+viewEnd)/2);}
    else if(e.key==='ArrowUp'){e.preventDefault();layerBack();}
    else if(e.key==='Escape'){e.preventDefault();setLayer('overview');}
  });
}
function mouseToMs(e) { var tl=document.getElementById('timeline'),rect=tl.getBoundingClientRect(),cL=rect.left+90,cW=rect.width-90,x=e.clientX-cL; if(x<0||cW<=0)return -1; return viewStart+(x/cW)*(viewEnd-viewStart); }
function panBy(d) { var ns=viewStart+d,ne=viewEnd+d; if(ns<0){ne-=ns;ns=0;} if(ne>totalMs){ns-=(ne-totalMs);ne=totalMs;} viewStart=Math.max(0,ns);viewEnd=Math.min(totalMs,ne); renderAllCharts();updateLayerUI(); }
function updateReadout(ms) { var el=document.getElementById('readout');if(!el)return; var p=[msToTime(ms)]; var f=trackFrameAt(ms); if(f)p.push('Y:'+f.y.toFixed(0)+' Sat:'+f.sa.toFixed(0)+' Diff:'+f.diff.toFixed(1)); var si=sceneAt(ms); if(si>=0)p.push('Scene '+si); el.textContent=p.join('  |  '); }

// ── Detail panel ──
function renderDetail(ms) {
  var si=sceneAt(ms),scene=si>=0?SCENES[si]:null,frame=trackFrameAt(ms);
  var vid=document.getElementById('videoPlayer'); if(vid){try{vid.currentTime=ms/1000;}catch(e){}}
  var cEl=document.getElementById('detailSignals');
  if(cEl){
    var h='<div style="font-family:var(--mono);font-size:12px;margin-bottom:8px;color:var(--text)">@ '+esc(msToTime(ms))+'</div>';
    if(frame){h+='<div class="gauge-panel"><div class="gauge-sec">FRAME-LEVEL</div>';h+=gaugeLine('LUMINANCE',frame.y,0,255,null,null,'');h+=gaugeLine('SATURATION',frame.sa,0,128,null,null,'');h+=gaugeLine('RANGE',frame.yx-frame.yn,0,255,null,null,'');h+=gaugeLine('FRAME DIFF',frame.diff,0,50,null,null,'');h+='</div>';}
    if(scene){h+='<div class="gauge-panel"><div class="gauge-sec">SCENE '+si+'</div>';if(scene.audio)h+=gaugeLine('LOUDNESS',scene.audio.input_i,-40,0,null,null,'LUFS');if(scene.track)h+=gaugeLine('MOTION',scene.track.motion_intensity,0,30,null,null,'');if(scene.blur)h+=gaugeLine('SHARPNESS',scene.blur.laplacian_variance,0,8000,null,null,'lv');if(scene.signalstats){h+=gaugeLine('STATIC',scene.signalstats.static_frame_ratio,0,1,null,null,'');h+=gaugeLine('DARK',scene.signalstats.dark_frame_ratio,0,1,null,null,'');}h+='</div>';
    if(scene.transcript&&scene.transcript.length){h+='<div style="margin-top:6px">';for(var t=0;t<scene.transcript.length;t++)h+='<div class="insp-transcript"><span class="transcript-time">'+msToTime(scene.transcript[t].start_ms)+'</span> '+esc(scene.transcript[t].text)+'</div>';h+='</div>';}
    }
    cEl.innerHTML=h;
  }
  var rEl=document.getElementById('detailRaw');
  if(rEl&&scene) rEl.innerHTML='<div style="margin-bottom:4px;font-weight:600;color:var(--text-sec)">Scene '+si+'</div><pre class="preview-code" style="max-height:180px;font-size:9px">'+syntaxHL(JSON.stringify(scene,null,2))+'</pre>';
  else if(rEl) rEl.innerHTML='';
}
function gaugeLine(label,val,min,max,lo,hi,unit){if(val==null)return'';var pct=Math.max(0,Math.min(100,((val-min)/(max-min))*100));var isWarn=(lo!=null&&val<lo)||(hi!=null&&val>hi);var fmt=Math.abs(val)<10?val.toFixed(2):Math.abs(val)<100?val.toFixed(1):Math.round(val);var h='<div class="'+(isWarn?'gauge warn':'gauge')+'"><span class="g-label">'+esc(label)+'</span><span class="g-track"><span class="g-fill" style="width:'+pct.toFixed(1)+'%"></span>';if(lo!=null)h+='<span class="g-zone-lo" style="width:'+(((lo-min)/(max-min))*100).toFixed(1)+'%"></span>';if(hi!=null){var hp=((hi-min)/(max-min))*100;h+='<span class="g-zone-hi" style="left:'+hp.toFixed(1)+'%;width:'+(100-hp).toFixed(1)+'%"></span>';}h+='</span><span class="g-val">'+esc(String(fmt))+'<span class="g-unit">'+esc(unit)+'</span></span></div>';return h;}

// ════════════════════════════════════════════════════════════════
// DASHBOARD
// ════════════════════════════════════════════════════════════════
function renderDashboard(){var el=document.getElementById('dashboardPanel');if(!el||typeof DASHBOARD==='undefined'||!DASHBOARD){if(el)el.style.display='none';return;}var g=DASHBOARD.global_stats||{},chips='';if(g.brightness&&g.brightness.mean!=null)chips+='<div class="dash-stat"><span class="dash-label">'+esc('Brightness')+'</span><span class="dash-val">'+esc(g.brightness.mean.toFixed(0)+' \u00b1'+g.brightness.std.toFixed(0))+'</span></div>';if(g.audio&&g.audio.lufs!=null)chips+='<div class="dash-stat"><span class="dash-label">'+esc('Loudness')+'</span><span class="dash-val">'+esc(g.audio.lufs.toFixed(1)+' LUFS')+'</span></div>';if(g.avsync&&g.avsync.offset_ms!=null)chips+='<div class="dash-stat"><span class="dash-label">'+esc('AV Sync')+'</span><span class="dash-val">'+esc(g.avsync.offset_ms+'ms')+'</span></div>';if(g.pacing)chips+='<div class="dash-stat"><span class="dash-label">'+esc('Pacing')+'</span><span class="dash-val">'+esc((g.pacing.shortest_ms/1000).toFixed(1)+'s\u2013'+(g.pacing.longest_ms/1000).toFixed(1)+'s')+'</span></div>';if(g.motion){var mc=[];for(var k in g.motion)mc.push(k+':'+g.motion[k]);chips+='<div class="dash-stat"><span class="dash-label">'+esc('Motion')+'</span><span class="dash-val">'+esc(mc.join(' '))+'</span></div>';}el.innerHTML='<div class="dash-stats">'+chips+'</div>';}

// ════════════════════════════════════════════════════════════════
// FINDER — preserved
// ════════════════════════════════════════════════════════════════
function renderFinderTree(){var el=document.getElementById('finderTree');var h='';function rn(node,depth){if(node.type!=='dir')return;h+='<div class="tree-item dir" style="padding-left:'+(8+depth*12)+'px" onclick="finderSelectDir(this,\''+esc(node.name)+'\')" data-name="'+esc(node.name)+'">'+esc(node.name)+'/</div>';if(node.children)for(var i=0;i<node.children.length;i++)if(node.children[i].type==='dir')rn(node.children[i],depth+1);}rn(FILE_TREE,0);el.innerHTML=h;}
var finderCurrentDir=null;
function finderSelectDir(el,name){document.querySelectorAll('.tree-item').forEach(function(t){t.classList.remove('active');});el.classList.add('active');finderCurrentDir=findNode(FILE_TREE,name);renderFinderList(finderCurrentDir);}
function findNode(tree,name){if(tree.name===name)return tree;if(tree.children)for(var i=0;i<tree.children.length;i++){var f=findNode(tree.children[i],name);if(f)return f;}return null;}
function renderFinderList(node){var el=document.getElementById('finderList');if(!node||!node.children){el.innerHTML='';return;}var h='';for(var i=0;i<node.children.length;i++){var c=node.children[i];if(c.type==='dir')h+='<div class="file-item" onclick="finderSelectDir(document.querySelector(\'.tree-item[data-name=\\x22'+esc(c.name)+'\\x22]\'),\''+esc(c.name)+'\')"><span class="file-name">\ud83d\udcc1 '+esc(c.name)+'/</span></div>';else h+='<div class="file-item" onclick="finderPreview(\''+esc(c.name)+'\',\''+esc(c.ext)+'\','+c.size+')"><span class="file-name">'+esc(c.name)+'</span><span class="file-size">'+fmtSize(c.size)+'</span></div>';}el.innerHTML=h;}
function finderPreview(name,ext,size){var el=document.getElementById('finderPreview');document.querySelectorAll('.file-item').forEach(function(f){f.classList.remove('active');});event.currentTarget.classList.add('active');var node=findFileNode(FILE_TREE,name),content=node?node.content:null;if(ext==='png'||ext==='jpg'||ext==='jpeg'){var path=findFilePath(FILE_TREE,name);el.innerHTML='<div class="preview-header">'+esc(name)+' <span class="preview-size">'+fmtSize(size)+'</span></div><img class="preview-img" src="'+esc(path||name)+'">';}else if(ext==='json'&&content)el.innerHTML='<div class="preview-header">'+esc(name)+' <span class="preview-size">'+fmtSize(size)+'</span></div><pre class="preview-code">'+syntaxHL(jsonPretty(content))+'</pre>';else if(ext==='jsonl'&&content){var lines=content.split('\n').filter(function(l){return l.trim();}).slice(0,50);el.innerHTML='<div class="preview-header">'+esc(name)+' <span class="preview-size">'+fmtSize(size)+' \u00b7 '+lines.length+' lines</span></div><pre class="preview-code">'+syntaxHL(lines.map(function(l){return jsonPretty(l);}).join('\n---\n'))+'</pre>';}else if(ext==='md'&&content)el.innerHTML='<div class="preview-header">'+esc(name)+' <span class="preview-size">'+fmtSize(size)+'</span></div><div class="preview-md">'+renderMd(content)+'</div>';else if(content)el.innerHTML='<div class="preview-header">'+esc(name)+' <span class="preview-size">'+fmtSize(size)+'</span></div><pre class="preview-code">'+esc(content)+'</pre>';else el.innerHTML='<div class="preview-header">'+esc(name)+' <span class="preview-size">'+fmtSize(size)+'</span></div><div class="finder-empty">Binary or too large</div>';}
function findFileNode(tree,name){if(tree.type==='file'&&tree.name===name&&tree.content)return tree;if(tree.children)for(var i=0;i<tree.children.length;i++){var f=findFileNode(tree.children[i],name);if(f)return f;}return null;}
function findFilePath(tree,name,prefix){prefix=prefix||'';if(tree.type==='file'&&tree.name===name)return prefix+name;if(tree.children)for(var i=0;i<tree.children.length;i++){var p=findFilePath(tree.children[i],name,prefix+tree.name+'/');if(p)return p.replace('observation/','');}return null;}
function jsonPretty(raw){try{return JSON.stringify(JSON.parse(raw),null,2);}catch(e){return raw;}}
function syntaxHL(text){return esc(text).replace(/("(?:\\.|[^"\\])*")\s*:/g,'<span class="hl-key">$1</span>:').replace(/:(\s*)("(?:\\.|[^"\\])*")/g,':<span class="hl-str">$2</span>').replace(/:\s*(-?\d+\.?\d*)/g,': <span class="hl-num">$1</span>').replace(/:\s*(true|false|null)/g,': <span class="hl-kw">$1</span>');}
function renderMd(text){var lines=text.split('\n'),html='',inCode=false;for(var i=0;i<lines.length;i++){var l=lines[i];if(l.startsWith('```')){if(inCode){html+='</pre>';inCode=false;}else{html+='<pre class="md-code">';inCode=true;}continue;}if(inCode){html+=esc(l)+'\n';continue;}if(l.startsWith('# ')){html+='<h1 class="md-h1">'+esc(l.slice(2))+'</h1>';continue;}if(l.startsWith('## ')){html+='<h2 class="md-h2">'+esc(l.slice(3))+'</h2>';continue;}if(l.startsWith('### ')){html+='<h3 class="md-h3">'+esc(l.slice(4))+'</h3>';continue;}if(l.startsWith('- ')){html+='<div class="md-li">\u2022 '+esc(l.slice(2))+'</div>';continue;}if(l.trim()===''){html+='<br>';continue;}html+='<p class="md-p">'+esc(l).replace(/\*\*([^*]+)\*\*/g,'<strong>$1</strong>').replace(/`([^`]+)`/g,'<code class="md-inline">$1</code>')+'</p>';}if(inCode)html+='</pre>';return html;}
function onSearch(){}

// ════════════════════════════════════════════════════════════════
// KEYFRAME STRIP — renders fixed-interval keyframe thumbnails
// SECURITY NOTE: All data is locally generated sensor output. No external input.
// ════════════════════════════════════════════════════════════════
function renderKeyframeStrip(vS, vE) {
  var el = document.getElementById('keyframeStrip'); if (!el) return;
  if (typeof KEYFRAMES_TRACK === 'undefined' || !KEYFRAMES_TRACK || !KEYFRAMES_TRACK.length) {
    el.textContent = 'No keyframes track';
    return;
  }
  // Clear previous content safely
  while (el.firstChild) el.removeChild(el.firstChild);
  var containerWidth = el.clientWidth || 900;
  var visible = [];
  for (var i = 0; i < KEYFRAMES_TRACK.length; i++) {
    var kf = KEYFRAMES_TRACK[i];
    if (kf.t >= vS && kf.t <= vE) visible.push(kf);
  }
  if (!visible.length) return;
  var maxThumbs = Math.min(visible.length, Math.floor(containerWidth / 24));
  var step = Math.max(1, Math.floor(visible.length / maxThumbs));
  var thumbW = Math.max(24, Math.floor(containerWidth / Math.ceil(visible.length / step)));
  for (var i = 0; i < visible.length; i += step) {
    var kf = visible[i];
    var img = document.createElement('img');
    img.className = 'kf-thumb';
    img.src = 'tracks/' + kf.path;
    img.width = thumbW;
    img.title = msToTime(kf.t);
    img.setAttribute('data-t', kf.t);
    img.onclick = function() { var t = parseInt(this.getAttribute('data-t')); selectedMs = t; renderDetail(t); drillTo('1s', t); };
    el.appendChild(img);
  }
  var rangeEl = document.getElementById('rangeKeyframes');
  if (rangeEl) rangeEl.textContent = visible.length + ' frames \u00b7 2fps';
}

// ════════════════════════════════════════════════════════════════
// SENSOR CONFIG PANEL — shows what sensors ran and their sampling config
// SECURITY NOTE: All data is locally generated sensor output. No external input.
// ════════════════════════════════════════════════════════════════
function renderSensorConfig() {
  var el = document.getElementById('sensorConfigPanel'); if (!el) return;
  if (typeof SENSOR_CONFIG === 'undefined' || !SENSOR_CONFIG || !SENSOR_CONFIG.length) {
    el.style.display = 'none'; return;
  }
  while (el.firstChild) el.removeChild(el.firstChild);
  for (var i = 0; i < SENSOR_CONFIG.length; i++) {
    var s = SENSOR_CONFIG[i];
    var detail = '';
    if (s.type === 'track') {
      detail = s.frames + ' frames';
      if (s.fps) detail += ' \u00b7 ' + s.fps + 'fps';
      if (s.interval_ms) detail += ' \u00b7 ' + s.interval_ms + 'ms';
    } else if (s.type === 'frames') {
      detail = s.count + ' keyframes \u00b7 ' + s.fps + 'fps \u00b7 ' + s.interval_ms + 'ms';
    } else if (s.type === 'signal') {
      detail = (s.size > 1024 ? (s.size / 1024).toFixed(0) + 'KB' : s.size + 'B');
    }
    var chip = document.createElement('div');
    chip.className = 'sensor-chip';
    var dot = document.createElement('span');
    dot.className = 'sc-dot';
    chip.appendChild(dot);
    chip.appendChild(document.createTextNode(s.id + ' '));
    var dim = document.createElement('span');
    dim.className = 'sc-dim';
    dim.textContent = detail;
    chip.appendChild(dim);
    el.appendChild(chip);
  }
}

// ════════════════════════════════════════════════════════════════
// INFO TOGGLE — shows description bubbles on all signal lanes
// ════════════════════════════════════════════════════════════════
function toggleInfo() {
  var active = document.body.classList.toggle('info-active');
  var hint = document.getElementById('infoHint');
  var btn = document.getElementById('btnInfo');
  if (hint) hint.textContent = active ? 'Hover over any section to see details' : '';
  if (btn) btn.textContent = active ? 'Quit info mode' : '\u2139';
}

// ── Init ──
renderDashboard();
renderSensorConfig();
renderTimeline();
