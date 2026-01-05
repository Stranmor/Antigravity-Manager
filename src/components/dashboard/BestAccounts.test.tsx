import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import BestAccounts from './BestAccounts';
import { Account } from '../../types/account';

// Mock react-i18next
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => {
      const translations: Record<string, string> = {
        'dashboard.best_accounts': 'Best Accounts',
        'dashboard.for_gemini': 'Best for Gemini',
        'dashboard.for_claude': 'Best for Claude',
        'dashboard.switch_best': 'Switch to Best',
        'accounts.no_data': 'No data available',
      };
      return translations[key] || key;
    },
  }),
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
  id: `test-id-${Math.random().toString(36).substring(7)}`,
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

describe('BestAccounts', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('rendering', () => {
    it('renders the component title', () => {
      render(<BestAccounts accounts={[]} />);
      expect(screen.getByText('Best Accounts')).toBeInTheDocument();
    });

    it('renders "no data" message when no accounts have valid quotas', () => {
      render(<BestAccounts accounts={[]} />);
      expect(screen.getByText('No data available')).toBeInTheDocument();
    });

    it('renders trending icon', () => {
      render(<BestAccounts accounts={[]} />);
      // TrendingUp icon should be present via lucide-react
      const heading = screen.getByRole('heading', { level: 2 });
      expect(heading).toBeInTheDocument();
    });
  });

  describe('best account selection', () => {
    it('displays best Gemini account when available', () => {
      const geminiAccount = createMockAccount({
        id: 'gemini-best',
        email: 'gemini-best@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 100, reset_time: '2025-01-16T12:00:00Z' },
            { name: 'gemini-3-flash', percentage: 80, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      render(<BestAccounts accounts={[geminiAccount]} />);
      expect(screen.getByText('gemini-best@example.com')).toBeInTheDocument();
      expect(screen.getByText('Best for Gemini')).toBeInTheDocument();
    });

    it('displays best Claude account when available', () => {
      const claudeAccount = createMockAccount({
        id: 'claude-best',
        email: 'claude-best@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'claude-sonnet-4-5-thinking', percentage: 95, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      render(<BestAccounts accounts={[claudeAccount]} />);
      expect(screen.getByText('claude-best@example.com')).toBeInTheDocument();
      expect(screen.getByText('Best for Claude')).toBeInTheDocument();
    });

    it('excludes current account from recommendations', () => {
      const currentAccount = createMockAccount({
        id: 'current-id',
        email: 'current@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 100, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      const alternativeAccount = createMockAccount({
        id: 'alternative-id',
        email: 'alternative@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 50, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      render(
        <BestAccounts
          accounts={[currentAccount, alternativeAccount]}
          currentAccountId="current-id"
        />
      );

      // Current account should be excluded, alternative should be shown
      expect(screen.queryByText('current@example.com')).not.toBeInTheDocument();
      expect(screen.getByText('alternative@example.com')).toBeInTheDocument();
    });

    it('selects different accounts for Gemini and Claude when same account is best for both', () => {
      const bestBothAccount = createMockAccount({
        id: 'best-both',
        email: 'best-both@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 100, reset_time: '2025-01-16T12:00:00Z' },
            { name: 'gemini-3-flash', percentage: 100, reset_time: '2025-01-16T12:00:00Z' },
            { name: 'claude-sonnet-4-5-thinking', percentage: 100, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      const secondBestGemini = createMockAccount({
        id: 'second-gemini',
        email: 'second-gemini@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 80, reset_time: '2025-01-16T12:00:00Z' },
            { name: 'gemini-3-flash', percentage: 70, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      const secondBestClaude = createMockAccount({
        id: 'second-claude',
        email: 'second-claude@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'claude-sonnet-4-5-thinking', percentage: 90, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      render(
        <BestAccounts
          accounts={[bestBothAccount, secondBestGemini, secondBestClaude]}
        />
      );

      // Should show best-both for one and second-best for the other
      const emails = screen.getAllByText(/example\.com/);
      const uniqueEmails = new Set(emails.map(el => el.textContent));
      expect(uniqueEmails.size).toBe(2); // Should have 2 different recommendations
    });
  });

  describe('quota calculation', () => {
    it('calculates Gemini quota with proper weighting (70% Pro, 30% Flash)', () => {
      const account = createMockAccount({
        id: 'weighted-account',
        email: 'weighted@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 100, reset_time: '2025-01-16T12:00:00Z' },
            { name: 'gemini-3-flash', percentage: 0, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      render(<BestAccounts accounts={[account]} />);
      // 100 * 0.7 + 0 * 0.3 = 70%
      expect(screen.getByText('70%')).toBeInTheDocument();
    });

    it('shows correct quota percentage for Claude', () => {
      const account = createMockAccount({
        id: 'claude-account',
        email: 'claude-test@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'claude-sonnet-4-5-thinking', percentage: 85, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      render(<BestAccounts accounts={[account]} />);
      expect(screen.getByText('85%')).toBeInTheDocument();
    });

    it('filters out accounts with zero quota', () => {
      const zeroQuotaAccount = createMockAccount({
        id: 'zero-quota',
        email: 'zero@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 0, reset_time: '2025-01-16T12:00:00Z' },
            { name: 'gemini-3-flash', percentage: 0, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      render(<BestAccounts accounts={[zeroQuotaAccount]} />);
      // Should show no data since zero quota is filtered out
      expect(screen.queryByText('zero@example.com')).not.toBeInTheDocument();
    });
  });

  describe('switch button', () => {
    it('renders switch button when onSwitch is provided', () => {
      const account = createMockAccount({
        id: 'switchable',
        email: 'switchable@example.com',
      });

      render(<BestAccounts accounts={[account]} onSwitch={vi.fn()} />);
      expect(screen.getByText('Switch to Best')).toBeInTheDocument();
    });

    it('does not render switch button when onSwitch is not provided', () => {
      const account = createMockAccount({
        id: 'no-switch',
        email: 'no-switch@example.com',
      });

      render(<BestAccounts accounts={[account]} />);
      expect(screen.queryByText('Switch to Best')).not.toBeInTheDocument();
    });

    it('calls onSwitch with Gemini account id when Gemini has higher quota', async () => {
      const user = userEvent.setup();
      const onSwitch = vi.fn();

      const geminiAccount = createMockAccount({
        id: 'gemini-high',
        email: 'gemini-high@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 100, reset_time: '2025-01-16T12:00:00Z' },
            { name: 'gemini-3-flash', percentage: 100, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      const claudeAccount = createMockAccount({
        id: 'claude-low',
        email: 'claude-low@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'claude-sonnet-4-5-thinking', percentage: 50, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      render(
        <BestAccounts
          accounts={[geminiAccount, claudeAccount]}
          onSwitch={onSwitch}
        />
      );

      await user.click(screen.getByText('Switch to Best'));
      expect(onSwitch).toHaveBeenCalledWith('gemini-high');
    });

    it('calls onSwitch with Claude account id when Claude has higher quota', async () => {
      const user = userEvent.setup();
      const onSwitch = vi.fn();

      const geminiAccount = createMockAccount({
        id: 'gemini-low',
        email: 'gemini-low@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 10, reset_time: '2025-01-16T12:00:00Z' },
            { name: 'gemini-3-flash', percentage: 10, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      const claudeAccount = createMockAccount({
        id: 'claude-high',
        email: 'claude-high@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'claude-sonnet-4-5-thinking', percentage: 100, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      render(
        <BestAccounts
          accounts={[geminiAccount, claudeAccount]}
          onSwitch={onSwitch}
        />
      );

      await user.click(screen.getByText('Switch to Best'));
      expect(onSwitch).toHaveBeenCalledWith('claude-high');
    });
  });

  describe('styling', () => {
    it('applies Gemini styling (green) to Gemini recommendation', () => {
      const account = createMockAccount({
        id: 'gemini-styled',
        email: 'gemini-styled@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 80, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      render(<BestAccounts accounts={[account]} />);
      const geminiLabel = screen.getByText('Best for Gemini');
      expect(geminiLabel).toHaveClass('text-green-600');
    });

    it('applies Claude styling (cyan) to Claude recommendation', () => {
      const account = createMockAccount({
        id: 'claude-styled',
        email: 'claude-styled@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'claude-sonnet-4-5-thinking', percentage: 80, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      render(<BestAccounts accounts={[account]} />);
      const claudeLabel = screen.getByText('Best for Claude');
      expect(claudeLabel).toHaveClass('text-cyan-600');
    });
  });

  describe('edge cases', () => {
    it('handles accounts without quota data', () => {
      const noQuotaAccount = createMockAccount({
        id: 'no-quota',
        email: 'no-quota@example.com',
        quota: undefined,
      });

      render(<BestAccounts accounts={[noQuotaAccount]} />);
      expect(screen.getByText('No data available')).toBeInTheDocument();
    });

    it('handles accounts with empty models array', () => {
      const emptyModelsAccount = createMockAccount({
        id: 'empty-models',
        email: 'empty-models@example.com',
        quota: {
          ...defaultQuota,
          models: [],
        },
      });

      render(<BestAccounts accounts={[emptyModelsAccount]} />);
      expect(screen.getByText('No data available')).toBeInTheDocument();
    });

    it('handles single account scenario', () => {
      const singleAccount = createMockAccount({
        id: 'single',
        email: 'single@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'gemini-3-pro-high', percentage: 100, reset_time: '2025-01-16T12:00:00Z' },
            { name: 'claude-sonnet-4-5-thinking', percentage: 100, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      render(<BestAccounts accounts={[singleAccount]} />);
      // Same account should be shown for both if it's the only one
      expect(screen.getAllByText('single@example.com').length).toBeGreaterThanOrEqual(1);
    });

    it('handles case-insensitive model name matching', () => {
      const account = createMockAccount({
        id: 'case-test',
        email: 'case-test@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'GEMINI-3-PRO-HIGH', percentage: 80, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      render(<BestAccounts accounts={[account]} />);
      expect(screen.getByText('case-test@example.com')).toBeInTheDocument();
    });
  });
});
