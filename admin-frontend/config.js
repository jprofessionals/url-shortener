// Runtime configuration for the admin frontend.
// Auto-detects environment based on hostname.
(function() {
  const host = window.location.hostname;

  let apiBase;
  if (host === 'localhost' || host === '127.0.0.1') {
    apiBase = 'http://localhost:3001';
  } else if (host === 'dev-admin-sc.jpro.dev') {
    apiBase = 'https://dev-sc.jpro.dev';
  } else if (host === 'admin-sc.jpro.dev') {
    apiBase = 'https://sc.jpro.dev';
  } else {
    // Fallback: assume prod
    apiBase = 'https://sc.jpro.dev';
  }

  window.APP_CONFIG = {
    API_BASE: apiBase,
    GOOGLE_CLIENT_ID: "333449424444-bb173lfcpqurosj5o2b39lmkpovnceqi.apps.googleusercontent.com",
    AUTH_DISABLED: false
  };
})();
