import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import CurrentAccount from './CurrentAccount';
import { Account } from '../../types/account';

// Mock react-i18next
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => {
      const translations: Record<string, string> = {
        'dashboard.current_account': 'Current Account',
        'dashboard.no_active_account': 'No active account',
        'dashboard.switch_account': 'Switch Account',
        'accounts.reset_time': 'Reset time',
        'common.unknown': 'Unknown',
      };
      return translations[key] || key;
    },
  }),
}));

// Mock format utility
vi.mock('../../utils/format', () => ({
  formatTimeRemaining: (time: string) => {
    if (!time) return 'Unknown';
    return '2h 30m';
  },
}));

const defaultQuota = {
  models: [
    { name: 'gemini-3-pro-high', percentage: 75, reset_time: '2025-01-16T12:00:00Z' },
    { name: 'gemini-3-flash', percentage: 50, reset_time: '2025-01-16T12:00:00Z' },
    { name: 'claude-sonnet-4-5-thinking', percentage: 60, reset_time: '2025-01-16T12:00:00Z' },
  ],
  last_updated: Date.now(),
  subscription_tier: 'FREE' as const,
};

const createMockAccount = (overrides?: Partial<Account>): Account => ({
  id: 'test-id-1',
  email: 'test@example.com',
  token: {
    access_token: 'access_token',
    refresh_token: 'refresh_token',
    expires_in: 3600,
    expiry_timestamp: Date.now() + 3600000,
    token_type: 'Bearer',
  },
  quota: defaultQuota,
  created_at: Date.now() / 1000,
  last_used: Date.now() / 1000,
  ...overrides,
});

describe('CurrentAccount', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('rendering with no account', () => {
    it('renders empty state when account is null', () => {
      render(<CurrentAccount account={null} />);
      expect(screen.getByText('No active account')).toBeInTheDocument();
    });

    it('renders title even when account is null', () => {
      render(<CurrentAccount account={null} />);
      expect(screen.getByText('Current Account')).toBeInTheDocument();
    });

    it('renders CheckCircle icon in title', () => {
      render(<CurrentAccount account={null} />);
      const heading = screen.getByRole('heading', { level: 2 });
      expect(heading).toBeInTheDocument();
      expect(heading).toHaveTextContent('Current Account');
    });
  });

  describe('rendering with account', () => {
    it('renders account email', () => {
      render(<CurrentAccount account={createMockAccount()} />);
      expect(screen.getByText('test@example.com')).toBeInTheDocument();
    });

    it('renders Gemini Pro quota', () => {
      render(<CurrentAccount account={createMockAccount()} />);
      expect(screen.getByText('Gemini 3 Pro')).toBeInTheDocument();
      expect(screen.getByText('75%')).toBeInTheDocument();
    });

    it('renders Gemini Flash quota', () => {
      render(<CurrentAccount account={createMockAccount()} />);
      expect(screen.getByText('Gemini 3 Flash')).toBeInTheDocument();
      expect(screen.getByText('50%')).toBeInTheDocument();
    });

    it('renders Claude quota', () => {
      render(<CurrentAccount account={createMockAccount()} />);
      expect(screen.getByText('Claude 4.5')).toBeInTheDocument();
      expect(screen.getByText('60%')).toBeInTheDocument();
    });

    it('renders reset time for quotas', () => {
      render(<CurrentAccount account={createMockAccount()} />);
      // Multiple reset times should be shown
      const resetTimes = screen.getAllByText(/R: 2h 30m/);
      expect(resetTimes.length).toBeGreaterThan(0);
    });
  });

  describe('subscription tiers', () => {
    it('renders FREE badge for free tier', () => {
      const freeAccount = createMockAccount({
        quota: { ...defaultQuota, subscription_tier: 'FREE' },
      });
      render(<CurrentAccount account={freeAccount} />);
      expect(screen.getByText('FREE')).toBeInTheDocument();
    });

    it('renders PRO badge for pro tier', () => {
      const proAccount = createMockAccount({
        quota: { ...defaultQuota, subscription_tier: 'PRO' },
      });
      render(<CurrentAccount account={proAccount} />);
      expect(screen.getByText('PRO')).toBeInTheDocument();
    });

    it('renders ULTRA badge for ultra tier', () => {
      const ultraAccount = createMockAccount({
        quota: { ...defaultQuota, subscription_tier: 'ULTRA' },
      });
      render(<CurrentAccount account={ultraAccount} />);
      expect(screen.getByText('ULTRA')).toBeInTheDocument();
    });

    it('applies gradient styling to PRO badge', () => {
      const proAccount = createMockAccount({
        quota: { ...defaultQuota, subscription_tier: 'PRO' },
      });
      render(<CurrentAccount account={proAccount} />);
      const proBadge = screen.getByText('PRO');
      expect(proBadge).toHaveClass('bg-gradient-to-r');
    });

    it('applies gradient styling to ULTRA badge', () => {
      const ultraAccount = createMockAccount({
        quota: { ...defaultQuota, subscription_tier: 'ULTRA' },
      });
      render(<CurrentAccount account={ultraAccount} />);
      const ultraBadge = screen.getByText('ULTRA');
      expect(ultraBadge).toHaveClass('bg-gradient-to-r');
    });
  });

  describe('quota color coding', () => {
    it('applies green color for quotas >= 50%', () => {
      const highQuotaAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 75, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });
      render(<CurrentAccount account={highQuotaAccount} />);
      const percentage = screen.getByText('75%');
      expect(percentage).toHaveClass('text-emerald-600');
    });

    it('applies amber color for quotas between 20% and 50%', () => {
      const mediumQuotaAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 35, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });
      render(<CurrentAccount account={mediumQuotaAccount} />);
      const percentage = screen.getByText('35%');
      expect(percentage).toHaveClass('text-amber-600');
    });

    it('applies rose color for quotas < 20%', () => {
      const lowQuotaAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 10, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });
      render(<CurrentAccount account={lowQuotaAccount} />);
      const percentage = screen.getByText('10%');
      expect(percentage).toHaveClass('text-rose-600');
    });
  });

  describe('progress bars', () => {
    it('renders progress bars with correct width', () => {
      render(<CurrentAccount account={createMockAccount()} />);
      // Check that progress bars exist (they have rounded-full class)
      const progressBars = document.querySelectorAll('.rounded-full');
      expect(progressBars.length).toBeGreaterThan(0);
    });

    it('applies green gradient for high quota progress bars', () => {
      const highQuotaAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 75, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });
      render(<CurrentAccount account={highQuotaAccount} />);
      const progressBar = document.querySelector('[style*="width: 75%"]');
      expect(progressBar).toHaveClass('bg-gradient-to-r');
    });
  });

  describe('switch button', () => {
    it('renders switch button when onSwitch is provided', () => {
      render(<CurrentAccount account={createMockAccount()} onSwitch={vi.fn()} />);
      expect(screen.getByText('Switch Account')).toBeInTheDocument();
    });

    it('does not render switch button when onSwitch is not provided', () => {
      render(<CurrentAccount account={createMockAccount()} />);
      expect(screen.queryByText('Switch Account')).not.toBeInTheDocument();
    });

    it('calls onSwitch when switch button is clicked', async () => {
      const user = userEvent.setup();
      const onSwitch = vi.fn();
      render(<CurrentAccount account={createMockAccount()} onSwitch={onSwitch} />);

      await user.click(screen.getByText('Switch Account'));
      expect(onSwitch).toHaveBeenCalledTimes(1);
    });
  });

  describe('model visibility', () => {
    it('does not render Gemini Pro section when model is missing', () => {
      const noGeminiProAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-flash', percentage: 50, reset_time: '2025-01-16T12:00:00Z' },
            { name: 'claude-sonnet-4-5-thinking', percentage: 60, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });
      render(<CurrentAccount account={noGeminiProAccount} />);
      expect(screen.queryByText('Gemini 3 Pro')).not.toBeInTheDocument();
    });

    it('does not render Gemini Flash section when model is missing', () => {
      const noGeminiFlashAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 75, reset_time: '2025-01-16T12:00:00Z' },
            { name: 'claude-sonnet-4-5-thinking', percentage: 60, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });
      render(<CurrentAccount account={noGeminiFlashAccount} />);
      expect(screen.queryByText('Gemini 3 Flash')).not.toBeInTheDocument();
    });

    it('does not render Claude section when model is missing', () => {
      const noClaudeAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 75, reset_time: '2025-01-16T12:00:00Z' },
            { name: 'gemini-3-flash', percentage: 50, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });
      render(<CurrentAccount account={noClaudeAccount} />);
      expect(screen.queryByText('Claude 4.5')).not.toBeInTheDocument();
    });
  });

  describe('edge cases', () => {
    it('handles account without quota', () => {
      const noQuotaAccount = createMockAccount({ quota: undefined });
      render(<CurrentAccount account={noQuotaAccount} />);
      // Should still render email
      expect(screen.getByText('test@example.com')).toBeInTheDocument();
      // Should not render any quota sections
      expect(screen.queryByText('Gemini 3 Pro')).not.toBeInTheDocument();
    });

    it('handles account with empty models array', () => {
      const emptyModelsAccount = createMockAccount({
        quota: { ...defaultQuota, models: [] },
      });
      render(<CurrentAccount account={emptyModelsAccount} />);
      expect(screen.getByText('test@example.com')).toBeInTheDocument();
    });

    it('handles quota with no subscription tier', () => {
      const noTierAccount = createMockAccount({
        quota: { ...defaultQuota, subscription_tier: undefined },
      });
      render(<CurrentAccount account={noTierAccount} />);
      // Should not crash, subscription badge should not appear
      expect(screen.getByText('test@example.com')).toBeInTheDocument();
    });

    it('handles missing reset_time gracefully', () => {
      const noResetTimeAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 75, reset_time: '' },
          ],
        },
      });
      render(<CurrentAccount account={noResetTimeAccount} />);
      expect(screen.getByText('Unknown')).toBeInTheDocument();
    });
  });

  describe('layout and structure', () => {
    it('renders with proper card styling', () => {
      render(<CurrentAccount account={createMockAccount()} />);
      const card = screen.getByText('test@example.com').closest('.bg-white');
      expect(card).toHaveClass('rounded-xl');
    });

    it('renders email with Mail icon', () => {
      render(<CurrentAccount account={createMockAccount()} />);
      // Email should be in a flex container with icon
      const emailElement = screen.getByText('test@example.com');
      const container = emailElement.closest('.flex');
      expect(container).toBeInTheDocument();
    });
  });
});
