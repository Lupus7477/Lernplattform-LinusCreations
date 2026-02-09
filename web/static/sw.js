// Service Worker für Offline-Unterstützung
const CACHE_NAME = 'lernplattform-v1';
const STATIC_CACHE = 'lernplattform-static-v1';
const API_CACHE = 'lernplattform-api-v1';

// Statische Assets die immer gecacht werden sollen
const STATIC_ASSETS = [
  '/',
  '/index.html',
  '/css/style.css',
  '/js/app.js',
  '/js/marked.min.js',
  '/js/highlight.min.js',
  '/css/github.min.css',
  '/manifest.json'
];

// API-Routen die gecacht werden können
const CACHEABLE_API_ROUTES = [
  '/api/v1/status',
  '/api/v1/documents',
  '/api/v1/plans/active',
  '/api/v1/glossary',
  '/api/v1/progress'
];

// Install Event - Cache statische Assets
self.addEventListener('install', (event) => {
  console.log('[SW] Installing Service Worker...');
  event.waitUntil(
    caches.open(STATIC_CACHE)
      .then((cache) => {
        console.log('[SW] Caching static assets');
        return cache.addAll(STATIC_ASSETS);
      })
      .then(() => self.skipWaiting())
      .catch((err) => console.log('[SW] Cache error:', err))
  );
});

// Activate Event - Cleanup alte Caches
self.addEventListener('activate', (event) => {
  console.log('[SW] Activating Service Worker...');
  event.waitUntil(
    caches.keys().then((cacheNames) => {
      return Promise.all(
        cacheNames
          .filter((name) => name !== STATIC_CACHE && name !== API_CACHE)
          .map((name) => {
            console.log('[SW] Deleting old cache:', name);
            return caches.delete(name);
          })
      );
    }).then(() => self.clients.claim())
  );
});

// Fetch Event - Intelligentes Caching
self.addEventListener('fetch', (event) => {
  const { request } = event;
  const url = new URL(request.url);

  // Nur GET-Requests cachen
  if (request.method !== 'GET') {
    return;
  }

  // API-Requests: Network First, dann Cache
  if (url.pathname.startsWith('/api/')) {
    event.respondWith(networkFirstStrategy(request));
    return;
  }

  // Statische Assets: Cache First, dann Network
  event.respondWith(cacheFirstStrategy(request));
});

// Cache First Strategie (für statische Assets)
async function cacheFirstStrategy(request) {
  const cachedResponse = await caches.match(request);
  
  if (cachedResponse) {
    // Im Hintergrund aktualisieren (Stale-While-Revalidate)
    fetchAndCache(request, STATIC_CACHE);
    return cachedResponse;
  }

  try {
    const networkResponse = await fetch(request);
    if (networkResponse.ok) {
      const cache = await caches.open(STATIC_CACHE);
      cache.put(request, networkResponse.clone());
    }
    return networkResponse;
  } catch (error) {
    // Offline Fallback
    return caches.match('/index.html');
  }
}

// Network First Strategie (für API-Calls)
async function networkFirstStrategy(request) {
  const url = new URL(request.url);
  const isCacheable = CACHEABLE_API_ROUTES.some(route => url.pathname.includes(route));

  try {
    const networkResponse = await fetch(request);
    
    // Cache erfolgreiche API-Responses
    if (networkResponse.ok && isCacheable) {
      const cache = await caches.open(API_CACHE);
      cache.put(request, networkResponse.clone());
    }
    
    return networkResponse;
  } catch (error) {
    // Fallback auf Cache wenn offline
    const cachedResponse = await caches.match(request);
    if (cachedResponse) {
      console.log('[SW] Serving from cache:', request.url);
      return cachedResponse;
    }
    
    // Offline-Fehler zurückgeben
    return new Response(
      JSON.stringify({ 
        error: 'Offline - Keine Verbindung zum Server',
        offline: true 
      }),
      { 
        status: 503,
        headers: { 'Content-Type': 'application/json' }
      }
    );
  }
}

// Hintergrund-Aktualisierung
async function fetchAndCache(request, cacheName) {
  try {
    const networkResponse = await fetch(request);
    if (networkResponse.ok) {
      const cache = await caches.open(cacheName);
      cache.put(request, networkResponse);
    }
  } catch (error) {
    // Ignore - wir haben ja schon den Cache
  }
}

// Message Handler für manuelle Cache-Kontrolle
self.addEventListener('message', (event) => {
  if (event.data === 'skipWaiting') {
    self.skipWaiting();
  }
  
  if (event.data === 'clearCache') {
    caches.keys().then((names) => {
      names.forEach((name) => caches.delete(name));
    });
  }
});
