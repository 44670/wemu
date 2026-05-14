var CACHE_NAME = 'v14';


var urlsToCache = [
    '/',
    '/index.html',
    '/player/',
    '/player/index.html',
    '/wemu.js',
    '/wemu.wasm',
    '/manifest.webmanifest',
    '/icons/wemu-icon-180.png',
    '/icons/wemu-icon-192.png',
    '/icons/wemu-icon-512.png'
];

self.addEventListener('install', function (event) {
    postMsg({msg:'Updating...'});
    var urlsAddVersion = urlsToCache.map(function (url) {
        return url + '?ver=' + CACHE_NAME
    });
    // Perform install steps
    event.waitUntil(
        caches.open(CACHE_NAME)
            .then(function (cache) {
                console.log('Opened cache');
                return cache.addAll(urlsAddVersion);
            }).then(() => {
                console.log('Cache downloaded')
                self.skipWaiting()
            })
    );
});


self.addEventListener('fetch', function (event) {
    event.respondWith(
        caches.match(event.request, {
            ignoreSearch: true
        }).then(function (response) {
            // Cache hit - return response
            if (response) {
                return response;
            }
            console.log('cache miss', event.request.url)
            return fetch(event.request);
        })
    );
});


self.addEventListener('activate', function (event) {
    console.log('activated, remove unused cache...')
    var cacheAllowlist = [CACHE_NAME];
    event.waitUntil(
        caches.keys().then(function (cacheNames) {
            return Promise.all(
                cacheNames.map(function (cacheName) {
                    if (cacheAllowlist.indexOf(cacheName) === -1) {
                        console.log(cacheName)
                        return caches.delete(cacheName);
                    }
                })
            );
        })
    );
    postMsg({msg:'Updated!'})
});

function postMsg(obj) {
    clients.matchAll({ includeUncontrolled: true, type: 'window' }).then((arr) => {
        for (client of arr) {
            client.postMessage(obj);
        }
    })
}
