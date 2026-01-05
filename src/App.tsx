import { createBrowserRouter, RouterProvider } from 'react-router-dom';
import { lazy, Suspense } from 'react';

import Layout from './components/layout/Layout';
import ThemeManager from './components/common/ThemeManager';
import { useEffect } from 'react';
import { useConfigStore } from './stores/useConfigStore';
import { useAccountStore } from './stores/useAccountStore';
import { useTranslation } from 'react-i18next';
import { listen } from '@tauri-apps/api/event';

// Lazy load pages for code splitting
const Dashboard = lazy(() => import('./pages/Dashboard'));
const Accounts = lazy(() => import('./pages/Accounts'));
const Settings = lazy(() => import('./pages/Settings'));
const ApiProxy = lazy(() => import('./pages/ApiProxy'));
const Monitor = lazy(() => import('./pages/Monitor'));

// Loading fallback component
function PageLoader() {
  return (
    <div className="flex items-center justify-center h-full w-full">
      <div className="loading loading-spinner loading-lg text-blue-500" />
    </div>
  );
}

const router = createBrowserRouter([
  {
    path: '/',
    element: <Layout />,
    children: [
      {
        index: true,
        element: <Suspense fallback={<PageLoader />}><Dashboard /></Suspense>,
      },
      {
        path: 'accounts',
        element: <Suspense fallback={<PageLoader />}><Accounts /></Suspense>,
      },
      {
        path: 'api-proxy',
        element: <Suspense fallback={<PageLoader />}><ApiProxy /></Suspense>,
      },
      {
        path: 'monitor',
        element: <Suspense fallback={<PageLoader />}><Monitor /></Suspense>,
      },
      {
        path: 'settings',
        element: <Suspense fallback={<PageLoader />}><Settings /></Suspense>,
      },
    ],
  },
]);

function App() {
  const { config, loadConfig } = useConfigStore();
  const { fetchCurrentAccount, fetchAccounts } = useAccountStore();
  const { i18n } = useTranslation();

  useEffect(() => {
    void loadConfig();
  }, [loadConfig]);

  // Sync language from config
  useEffect(() => {
    if (config?.language) {
      void i18n.changeLanguage(config.language);
    }
  }, [config?.language, i18n]);

  // Listen for tray events
  useEffect(() => {
    const unlistenPromises: Promise<() => void>[] = [];

    // 监听托盘切换账号事件
    unlistenPromises.push(
      listen('tray://account-switched', () => {
        void fetchCurrentAccount();
        void fetchAccounts();
      })
    );

    // 监听托盘刷新事件
    unlistenPromises.push(
      listen('tray://refresh-current', () => {
        void fetchCurrentAccount();
        void fetchAccounts();
      })
    );

    // Cleanup
    return () => {
      void Promise.all(unlistenPromises).then(unlisteners => {
        unlisteners.forEach(unlisten => { unlisten(); });
      });
    };
  }, [fetchCurrentAccount, fetchAccounts]);

  return (
    <>
      <ThemeManager />
      <RouterProvider router={router} />
    </>
  );
}

export default App;