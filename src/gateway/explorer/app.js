/**
 * TKS Real-Time Substrate Web Explorer (WP-3.4, PHASE3-001)
 * Interactive Cytoscape.js DAG Canvas, State Inspector Drawer, and Live SSE Listener.
 */

(function () {
  'use strict';

  // Register Dagre layout extension with Cytoscape if available
  if (typeof cytoscapeDagre !== 'undefined' && typeof cytoscape !== 'undefined') {
    cytoscape.use(cytoscapeDagre);
  }

  // Application State
  let cy = null;
  let eventSource = null;
  let currentElements = { nodes: [], edges: [] };

  // DOM Elements
  const cyContainer = document.getElementById('cy');
  const searchBox = document.getElementById('search-box');
  const rootFilter = document.getElementById('root-filter');
  const depthSelect = document.getElementById('depth-select');
  const draftsToggle = document.getElementById('drafts-toggle');
  const layoutSelect = document.getElementById('layout-select');
  const fitBtn = document.getElementById('fit-btn');
  const resetBtn = document.getElementById('reset-btn');
  const statusPill = document.getElementById('status-pill');
  const nodeCountEl = document.getElementById('node-count');
  const edgeCountEl = document.getElementById('edge-count');
  const layoutNameEl = document.getElementById('layout-name');
  const eventFeed = document.getElementById('event-feed');

  // Drawer Elements
  const drawer = document.getElementById('drawer');
  const drawerCloseBtn = document.getElementById('drawer-close-btn');
  const drawerNodeKey = document.getElementById('drawer-node-key');
  const drawerTitle = document.getElementById('drawer-title');
  const drawerStateBadge = document.getElementById('drawer-state-badge');
  const drawerPolicyBadge = document.getElementById('drawer-policy-badge');
  const drawerStalenessAlert = document.getElementById('drawer-staleness-alert');
  const drawerStalenessScore = document.getElementById('drawer-staleness-score');
  const drawerContentSection = document.getElementById('drawer-content-section');
  const drawerContentText = document.getElementById('drawer-content-text');
  const drawerSpanSection = document.getElementById('drawer-span-section');
  const drawerSpanPath = document.getElementById('drawer-span-path');
  const drawerSpanOffsets = document.getElementById('drawer-span-offsets');
  const drawerSpanText = document.getElementById('drawer-span-text');
  const drawerAttributesJson = document.getElementById('drawer-attributes-json');

  // Policy icon mapping
  function getPolicyIcon(policy) {
    switch (policy) {
      case 'AUTONOMOUS_ELABORATION':
        return '⚡';
      case 'HUMAN_REVIEW_REQUIRED':
        return '👤';
      case 'LOCKED':
        return '🔒';
      default:
        return '•';
    }
  }

  // Cytoscape Stylesheet Specification (Task 4)
  const cytoscapeStyles = [
    {
      selector: 'node',
      style: {
        'label': function (ele) {
          const key = ele.data('node_key') || ele.data('id').substring(0, 8);
          const icon = getPolicyIcon(ele.data('governance_policy'));
          return `${icon} ${key}`;
        },
        'text-valign': 'center',
        'text-halign': 'center',
        'color': '#f8fafc',
        'font-family': 'ui-sans-serif, system-ui, sans-serif',
        'font-size': '12px',
        'font-weight': '600',
        'width': '120px',
        'height': '44px',
        'shape': 'round-rectangle',
        'border-width': '2px',
        'border-color': '#475569',
        'background-color': '#1e293b',
        'text-wrap': 'ellipsis',
        'text-max-width': '110px',
        'transition-property': 'background-color, border-color, border-width, opacity, shadow-opacity',
        'transition-duration': '0.3s'
      }
    },
    // Node Lifecycle States
    {
      selector: 'node[lifecycle_state = "ACTIVE"]',
      style: {
        'background-color': '#0d9488', // Teal/Green
        'border-color': '#14b8a6'
      }
    },
    {
      selector: 'node[lifecycle_state = "DRAFT"]',
      style: {
        'background-color': '#2563eb', // Blue
        'border-color': '#3b82f6',
        'border-style': 'dashed'
      }
    },
    {
      selector: 'node[lifecycle_state = "NEEDS_REVERIFICATION"]',
      style: {
        'background-color': '#d97706', // Amber/Orange
        'border-color': '#f59e0b',
        'border-width': '3px'
      }
    },
    {
      selector: 'node[lifecycle_state = "SUPERSEDED"]',
      style: {
        'background-color': '#4b5563', // Grey
        'border-color': '#6b7280',
        'opacity': 0.65
      }
    },
    // Governance Policy Badges & Outlines
    {
      selector: 'node[governance_policy = "LOCKED"]',
      style: {
        'border-style': 'double',
        'border-width': '4px'
      }
    },
    {
      selector: 'node[governance_policy = "HUMAN_REVIEW_REQUIRED"]',
      style: {
        'border-style': 'dashed'
      }
    },
    // Pulsing Animation state for live cascade sweeps
    {
      selector: 'node.pulsing',
      style: {
        'border-color': '#ef4444',
        'border-width': '6px',
        'background-color': '#b45309',
        'shadow-blur': '20px',
        'shadow-color': '#f59e0b',
        'shadow-opacity': 0.9
      }
    },
    // Node Selection
    {
      selector: 'node:selected',
      style: {
        'border-color': '#c084fc',
        'border-width': '4px',
        'shadow-blur': '16px',
        'shadow-color': '#a855f7',
        'shadow-opacity': 0.8
      }
    },
    // Edge Styles (Task 4)
    {
      selector: 'edge',
      style: {
        'width': 2,
        'curve-style': 'bezier',
        'target-arrow-shape': 'triangle',
        'arrow-scale': 1.2,
        'line-color': '#64748b',
        'target-arrow-color': '#64748b',
        'label': 'data(edge_type)',
        'font-family': 'ui-monospace, monospace',
        'font-size': '9px',
        'color': '#94a3b8',
        'text-rotation': 'autorotate',
        'text-margin-y': -8
      }
    },
    {
      selector: 'edge[edge_type = "FULFILLS"]',
      style: {
        'line-style': 'solid',
        'line-color': '#10b981',
        'target-arrow-color': '#10b981'
      }
    },
    {
      selector: 'edge[edge_type = "DERIVED_FROM"]',
      style: {
        'line-style': 'solid',
        'line-color': '#0284c7',
        'target-arrow-color': '#0284c7'
      }
    },
    {
      selector: 'edge[edge_type = "CONSTRAINED_BY"]',
      style: {
        'line-style': 'dashed',
        'line-color': '#ef4444',
        'target-arrow-color': '#ef4444'
      }
    }
  ];

  // Initialize Cytoscape Instance
  function initCytoscape() {
    cy = cytoscape({
      container: cyContainer,
      elements: [],
      style: cytoscapeStyles,
      boxSelectionEnabled: false,
      autounselectify: false,
      wheelSensitivity: 0.2
    });

    // Handle node click / tap
    cy.on('tap', 'node', function (evt) {
      const node = evt.target;
      inspectNode(node.data());
    });

    // Close drawer when clicking canvas background
    cy.on('tap', function (evt) {
      if (evt.target === cy) {
        closeDrawer();
      }
    });
  }

  // Fetch and Render Graph Topology
  async function loadGraph() {
    const root = rootFilter.value.trim();
    const depth = depthSelect.value;
    const includeDrafts = draftsToggle.checked;

    let url = `/api/v1/explorer/graph?depth=${depth}&include_drafts=${includeDrafts}`;
    if (root) {
      url += `&root=${encodeURIComponent(root)}`;
    }

    try {
      const res = await fetch(url);
      if (!res.ok) {
        if (res.status === 404) {
          alert(`Root node "${root}" not found.`);
        } else {
          console.error('Failed to load graph:', res.statusText);
        }
        return;
      }

      const data = await res.json();
      currentElements = data;

      // Update Cytoscape
      cy.elements().remove();
      cy.add(data.nodes);
      cy.add(data.edges);

      // Run configured layout
      applyLayout(layoutSelect.value);

      // Update Header Stats
      nodeCountEl.textContent = data.nodes ? data.nodes.length : 0;
      edgeCountEl.textContent = data.edges ? data.edges.length : 0;
      layoutNameEl.textContent = layoutSelect.options[layoutSelect.selectedIndex].text;
    } catch (err) {
      console.error('Error fetching graph data:', err);
    }
  }

  // Layout Runner
  function applyLayout(layoutName) {
    if (!cy) return;

    let layoutOpts = { name: layoutName, animate: true, animationDuration: 400 };

    if (layoutName === 'dagre') {
      try {
        layoutOpts = {
          name: 'dagre',
          rankDir: 'TB',
          nodeSep: 50,
          rankSep: 70,
          animate: true,
          animationDuration: 400
        };
      } catch (e) {
        console.warn('Dagre layout failed, falling back to breadthfirst:', e);
        layoutOpts = {
          name: 'breadthfirst',
          directed: true,
          spacingFactor: 1.5,
          animate: true
        };
      }
    } else if (layoutName === 'breadthfirst') {
      layoutOpts = {
        name: 'breadthfirst',
        directed: true,
        spacingFactor: 1.5,
        animate: true
      };
    } else if (layoutName === 'cose') {
      layoutOpts = {
        name: 'cose',
        idealEdgeLength: 100,
        nodeOverlap: 20,
        refresh: 20,
        fit: true,
        padding: 30,
        randomize: false,
        componentSpacing: 100,
        nodeRepulsion: 400000,
        edgeElasticity: 100,
        nestingFactor: 5,
        gravity: 80,
        numIter: 1000,
        initialTemp: 200,
        coolingFactor: 0.95,
        minTemp: 1.0
      };
    }

    const layout = cy.layout(layoutOpts);
    layout.run();
  }

  // Slide-Out Inspection Drawer
  async function inspectNode(data) {
    if (!data) return;

    drawerNodeKey.textContent = data.node_key || data.id;
    drawerTitle.textContent = data.title || '(Untitled)';

    // State badge
    const state = data.lifecycle_state || 'ACTIVE';
    drawerStateBadge.textContent = state;
    drawerStateBadge.className = `badge ${state.toLowerCase()}`;

    // Policy badge
    const policy = data.governance_policy || 'AUTONOMOUS_ELABORATION';
    const policyIcon = getPolicyIcon(policy);
    drawerPolicyBadge.textContent = `${policyIcon} ${policy.replace(/_/g, ' ')}`;
    drawerPolicyBadge.className = 'badge policy';

    // Staleness score alert
    const staleness = parseFloat(data.staleness_score) || 0.0;
    if (staleness > 0.0) {
      drawerStalenessScore.textContent = `Node is degraded (Staleness Score: ${staleness.toFixed(2)}). Descendant requirements require reverification.`;
      drawerStalenessAlert.style.display = 'flex';
    } else {
      drawerStalenessAlert.style.display = 'none';
    }

    // Attributes viewer
    drawerAttributesJson.textContent = JSON.stringify(data.attributes || {}, null, 2);

    // Initial clear of remote fields
    drawerContentSection.style.display = 'none';
    drawerSpanSection.style.display = 'none';

    // Fetch full node inspection details from REST API
    try {
      const res = await fetch(`/api/v1/nodes/${data.id}`);
      if (res.ok) {
        const fullDetails = await res.json();
        const node = fullDetails.node || fullDetails;

        if (node.content) {
          drawerContentText.textContent = node.content;
          drawerContentSection.style.display = 'block';
        }

        if (fullDetails.span_text) {
          drawerSpanPath.textContent = node.doc_path || 'Repository Document';
          drawerSpanOffsets.textContent = `[bytes ${node.byte_start || 0}..${node.byte_end || 0}]`;
          drawerSpanText.textContent = fullDetails.span_text;
          drawerSpanSection.style.display = 'block';
        }
      }
    } catch (err) {
      console.warn('Could not fetch remote inspection details:', err);
    }

    drawer.classList.add('open');
  }

  function closeDrawer() {
    drawer.classList.remove('open');
  }

  // Real-Time SSE Listener (Task 2 & Proof Criteria)
  function initSseListener() {
    if (eventSource) {
      eventSource.close();
    }

    eventSource = new EventSource('/api/v1/explorer/events');

    eventSource.onopen = function () {
      statusPill.textContent = '● LIVE SSE';
      statusPill.classList.remove('disconnected');
    };

    eventSource.onerror = function () {
      statusPill.textContent = '○ RECONNECTING...';
      statusPill.classList.add('disconnected');
    };

    const handleEvent = function (e) {
      try {
        const event = JSON.parse(e.data);
        handleGraphChangeEvent(event);
      } catch (err) {
        console.error('Failed to parse SSE JSON payload:', err);
      }
    };

    eventSource.addEventListener('graph_event', handleEvent);
    eventSource.onmessage = handleEvent;
  }

  // Handle incoming live GraphChangeEvent
  function handleGraphChangeEvent(event) {
    addEventToast(event);

    if (
      event.event_type === 'STAGING_APPROVED' ||
      event.event_type === 'WORKSPACE_PROMOTED' ||
      event.event_type === 'TASK_ELABORATED'
    ) {
      loadGraph();
      return;
    }

    const targetNodeId = event.entity_id;
    if (!targetNodeId || !cy) return;

    const cyNode = cy.getElementById(targetNodeId);

    if (cyNode && cyNode.length > 0) {
      // Node exists in canvas: trigger animated pulse effect
      cyNode.addClass('pulsing');
      setTimeout(() => {
        cyNode.removeClass('pulsing');
      }, 3000);

      if (event.event_type === 'CASCADE_INVALIDATED') {
        cyNode.data('lifecycle_state', 'NEEDS_REVERIFICATION');
      } else if (event.event_type === 'REVERIFIED') {
        cyNode.data('lifecycle_state', 'ACTIVE');
        cyNode.data('staleness_score', 0.0);
      } else if (event.event_type === 'TASK_STATUS_UPDATED') {
        loadGraph();
      }
    } else {
      loadGraph();
    }
  }

  // Add toast notification to live event feed
  function addEventToast(event) {
    const toast = document.createElement('div');
    toast.className = `event-toast ${
      event.event_type.includes('CASCADE') ? 'cascade' : 'mutation'
    }`;

    const timeStr = new Date(event.timestamp || Date.now()).toLocaleTimeString();
    toast.innerHTML = `
      <div class="event-toast-header">
        <span>${escapeHtml(event.event_type)}</span>
        <span class="time">${timeStr}</span>
      </div>
      <div>Entity: <code>${escapeHtml((event.entity_id || '').substring(0, 8))}</code> | Seq: #${event.event_seq}</div>
    `;

    eventFeed.prepend(toast);

    // Prune excess toasts
    while (eventFeed.children.length > 5) {
      eventFeed.removeChild(eventFeed.lastChild);
    }

    // Auto-remove after 8s
    setTimeout(() => {
      if (toast.parentNode === eventFeed) {
        eventFeed.removeChild(toast);
      }
    }, 8000);
  }

  function escapeHtml(str) {
    return String(str)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;');
  }

  // Search & Filter input handler
  function setupSearch() {
    searchBox.addEventListener('input', function () {
      const q = searchBox.value.trim().toLowerCase();
      if (!cy) return;

      if (!q) {
        cy.elements().style('opacity', 1);
        return;
      }

      cy.batch(() => {
        cy.nodes().each((node) => {
          const key = (node.data('node_key') || '').toLowerCase();
          const title = (node.data('title') || '').toLowerCase();
          if (key.includes(q) || title.includes(q)) {
            node.style('opacity', 1);
            node.connectedEdges().style('opacity', 1);
          } else {
            node.style('opacity', 0.15);
            node.connectedEdges().style('opacity', 0.1);
          }
        });
      });
    });
  }

  // Setup Event Listeners
  function setupEventListeners() {
    rootFilter.addEventListener('change', loadGraph);
    depthSelect.addEventListener('change', loadGraph);
    draftsToggle.addEventListener('change', loadGraph);

    layoutSelect.addEventListener('change', function () {
      applyLayout(layoutSelect.value);
      layoutNameEl.textContent = layoutSelect.options[layoutSelect.selectedIndex].text;
    });

    fitBtn.addEventListener('click', function () {
      if (cy) cy.fit(null, 40);
    });

    resetBtn.addEventListener('click', function () {
      searchBox.value = '';
      rootFilter.value = '';
      depthSelect.value = '3';
      draftsToggle.checked = false;
      layoutSelect.value = 'dagre';
      loadGraph();
    });

    drawerCloseBtn.addEventListener('click', closeDrawer);
  }

  // Application Entry Point
  window.addEventListener('DOMContentLoaded', function () {
    initCytoscape();
    setupEventListeners();
    setupSearch();
    loadGraph();
    initSseListener();
  });
})();
