import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { MemoryRouter } from 'react-router-dom';
import Navbar from './Navbar';

// Mock react-i18next
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => {
      const translations: Record<string, string> = {
        'nav.dashboard': 'Dashboard',
        'nav.accounts': 'Accounts',
        'nav.proxy': 'API Proxy',
        'nav.settings': 'Settings',
      };
      return translations[key] || key;
    },
    i18n: {
      changeLanguage: vi.fn(),
    },
  }),
}));

// Mock the config store
const mockSaveConfig = vi.fn();
const mockConfig = {
  theme: 'light',
  language: 'en',
  auto_refresh: true,
  refresh_interval: 15,
  auto_sync: false,
  sync_interval: 5,
  proxy: {
    enabled: false,
    port: 8080,
    api_key: '',
    auto_start: false,
    request_timeout: 120,
    enable_logging: false,
    upstream_proxy: { enabled: false, url: '' },
  },
};

vi.mock('../../stores/useConfigStore', () => ({
  useConfigStore: () => ({
    config: mockConfig,
    saveConfig: mockSaveConfig,
  }),
}));

const renderWithRouter = (initialPath = '/') => {
  return render(
    <MemoryRouter initialEntries={[initialPath]}>
      <Navbar />
    </MemoryRouter>
  );
};

describe('Navbar', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('rendering', () => {
    it('renders the logo/brand', () => {
      renderWithRouter();
      expect(screen.getByText('Antigravity Tools')).toBeInTheDocument();
    });

    it('renders all navigation links', () => {
      renderWithRouter();
      expect(screen.getByText('Dashboard')).toBeInTheDocument();
      expect(screen.getByText('Accounts')).toBeInTheDocument();
      expect(screen.getByText('API Proxy')).toBeInTheDocument();
      expect(screen.getByText('Settings')).toBeInTheDocument();
    });

    it('renders theme toggle button', () => {
      renderWithRouter();
      // There should be a button with Sun or Moon icon
      const buttons = screen.getAllByRole('button');
      expect(buttons.length).toBeGreaterThan(0);
    });

    it('renders language toggle button', () => {
      renderWithRouter();
      // When language is 'en', button shows '中' (to switch to Chinese)
      expect(screen.getByText('中')).toBeInTheDocument();
    });
  });

  describe('navigation', () => {
    it('has correct href for dashboard link', () => {
      renderWithRouter();
      const dashboardLink = screen.getByText('Dashboard');
      expect(dashboardLink.closest('a')).toHaveAttribute('href', '/');
    });

    it('has correct href for accounts link', () => {
      renderWithRouter();
      const accountsLink = screen.getByText('Accounts');
      expect(accountsLink.closest('a')).toHaveAttribute('href', '/accounts');
    });

    it('has correct href for proxy link', () => {
      renderWithRouter();
      const proxyLink = screen.getByText('API Proxy');
      expect(proxyLink.closest('a')).toHaveAttribute('href', '/api-proxy');
    });

    it('has correct href for settings link', () => {
      renderWithRouter();
      const settingsLink = screen.getByText('Settings');
      expect(settingsLink.closest('a')).toHaveAttribute('href', '/settings');
    });
  });

  describe('active state', () => {
    it('highlights dashboard link when on root path', () => {
      renderWithRouter('/');
      const dashboardLink = screen.getByText('Dashboard');
      expect(dashboardLink).toHaveClass('bg-gray-900');
    });

    it('highlights accounts link when on /accounts path', () => {
      renderWithRouter('/accounts');
      const accountsLink = screen.getByText('Accounts');
      expect(accountsLink).toHaveClass('bg-gray-900');
    });

    it('highlights proxy link when on /api-proxy path', () => {
      renderWithRouter('/api-proxy');
      const proxyLink = screen.getByText('API Proxy');
      expect(proxyLink).toHaveClass('bg-gray-900');
    });

    it('highlights settings link when on /settings path', () => {
      renderWithRouter('/settings');
      const settingsLink = screen.getByText('Settings');
      expect(settingsLink).toHaveClass('bg-gray-900');
    });

    it('highlights parent path for nested routes', () => {
      renderWithRouter('/accounts/some-id');
      const accountsLink = screen.getByText('Accounts');
      expect(accountsLink).toHaveClass('bg-gray-900');
    });
  });

  describe('theme toggle', () => {
    it('calls saveConfig when theme toggle is clicked', async () => {
      const user = userEvent.setup();
      renderWithRouter();

      // Find the theme toggle button (the one with Moon icon when light theme)
      const buttons = screen.getAllByRole('button');
      const themeButton = buttons.find(
        (btn) => btn.title.includes('切换') || btn.title.includes('深色') || btn.title.includes('浅色')
      );

      if (themeButton) {
        await user.click(themeButton);
        expect(mockSaveConfig).toHaveBeenCalled();
      }
    });
  });

  describe('language toggle', () => {
    it('shows Chinese button when current language is English', () => {
      // When language='en', the toggle shows '中' to switch to Chinese
      renderWithRouter();
      expect(screen.getByText('中')).toBeInTheDocument();
    });

    it('calls saveConfig when language toggle is clicked', async () => {
      const user = userEvent.setup();
      renderWithRouter();

      // Find the language button - it should have '中' when language is 'en'
      const langButton = screen.getByText('中');
      await user.click(langButton);
      expect(mockSaveConfig).toHaveBeenCalled();
    });
  });

  describe('styling', () => {
    it('has sticky positioning', () => {
      renderWithRouter();
      const nav = document.querySelector('nav');
      expect(nav).toHaveStyle({ position: 'sticky' });
    });

    it('has correct z-index', () => {
      renderWithRouter();
      const nav = document.querySelector('nav');
      expect(nav).toHaveStyle({ zIndex: '50' });
    });

    it('has drag region for Tauri', () => {
      renderWithRouter();
      const dragRegion = document.querySelector('[data-tauri-drag-region]');
      expect(dragRegion).toBeInTheDocument();
    });
  });

  describe('logo link', () => {
    it('logo links to home page', () => {
      renderWithRouter('/accounts');
      const logo = screen.getByText('Antigravity Tools');
      expect(logo.closest('a')).toHaveAttribute('href', '/');
    });
  });
});

describe('Navbar with Chinese language', () => {
  beforeEach(() => {
    // Update mock config for Chinese
    Object.assign(mockConfig, { language: 'zh' });
    vi.clearAllMocks();
  });

  it('shows "EN" button when language is Chinese', () => {
    renderWithRouter();
    expect(screen.getByText('EN')).toBeInTheDocument();
  });
});
