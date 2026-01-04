import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import AccountCard from './AccountCard';
import { Account } from '../../types/account';

// Mock react-i18next
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => {
      const translations: Record<string, string> = {
        'accounts.current': 'Current',
        'accounts.disabled': 'Disabled',
        'accounts.disabled_tooltip': 'This account is disabled',
        'accounts.forbidden': 'Forbidden',
        'accounts.forbidden_tooltip': 'This account is forbidden',
        'accounts.forbidden_msg': 'Account access is forbidden',
        'accounts.enable_proxy': 'Enable Proxy',
        'accounts.disable_proxy': 'Disable Proxy',
        'common.details': 'Details',
        'common.switch': 'Switch',
        'common.loading': 'Loading...',
        'common.refresh': 'Refresh',
        'common.export': 'Export',
        'common.delete': 'Delete',
      };
      return translations[key] || key;
    },
  }),
}));

// Default quota for reuse in tests
const defaultQuota = {
  models: [
    { name: 'gemini-3-pro-high', percentage: 75, reset_time: '2025-01-16T12:00:00Z' },
    { name: 'gemini-3-flash', percentage: 50, reset_time: '2025-01-16T12:00:00Z' },
    { name: 'gemini-3-pro-image', percentage: 25, reset_time: '2025-01-16T12:00:00Z' },
    { name: 'claude-sonnet-4-5-thinking', percentage: 10, reset_time: '2025-01-16T12:00:00Z' },
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

describe('AccountCard', () => {
  const defaultProps = {
    account: createMockAccount(),
    selected: false,
    onSelect: vi.fn(),
    isCurrent: false,
    isRefreshing: false,
    isSwitching: false,
    onSwitch: vi.fn(),
    onRefresh: vi.fn(),
    onViewDetails: vi.fn(),
    onExport: vi.fn(),
    onDelete: vi.fn(),
    onToggleProxy: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('rendering', () => {
    it('renders the account email', () => {
      render(<AccountCard {...defaultProps} />);
      expect(screen.getByText('test@example.com')).toBeInTheDocument();
    });

    it('renders current badge when isCurrent is true', () => {
      render(<AccountCard {...defaultProps} isCurrent />);
      expect(screen.getByText('CURRENT')).toBeInTheDocument();
    });

    it('does not render current badge when isCurrent is false', () => {
      render(<AccountCard {...defaultProps} isCurrent={false} />);
      expect(screen.queryByText('CURRENT')).not.toBeInTheDocument();
    });

    it('renders disabled badge when account is disabled', () => {
      const disabledAccount = createMockAccount({
        disabled: true,
        disabled_reason: 'Test reason',
      });
      render(<AccountCard {...defaultProps} account={disabledAccount} />);
      expect(screen.getByText('DISABLED')).toBeInTheDocument();
    });

    it('renders forbidden badge when quota is forbidden', () => {
      const forbiddenAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          is_forbidden: true,
        },
      });
      render(<AccountCard {...defaultProps} account={forbiddenAccount} />);
      expect(screen.getByText('FORBIDDEN')).toBeInTheDocument();
    });
  });

  describe('subscription tiers', () => {
    it('renders FREE badge for free tier', () => {
      render(<AccountCard {...defaultProps} />);
      expect(screen.getByText('FREE')).toBeInTheDocument();
    });

    it('renders PRO badge for pro tier', () => {
      const proAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          subscription_tier: 'PRO',
        },
      });
      render(<AccountCard {...defaultProps} account={proAccount} />);
      expect(screen.getByText('PRO')).toBeInTheDocument();
    });

    it('renders ULTRA badge for ultra tier', () => {
      const ultraAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          subscription_tier: 'ULTRA',
        },
      });
      render(<AccountCard {...defaultProps} account={ultraAccount} />);
      expect(screen.getByText('ULTRA')).toBeInTheDocument();
    });
  });

  describe('checkbox interaction', () => {
    it('checkbox reflects selected state', () => {
      render(<AccountCard {...defaultProps} selected />);
      const checkbox = screen.getByRole('checkbox');
      expect(checkbox).toBeChecked();
    });

    it('calls onSelect when checkbox is clicked', async () => {
      const user = userEvent.setup();
      const onSelect = vi.fn();
      render(<AccountCard {...defaultProps} onSelect={onSelect} />);

      await user.click(screen.getByRole('checkbox'));
      expect(onSelect).toHaveBeenCalledTimes(1);
    });
  });

  describe('action buttons', () => {
    it('calls onViewDetails when details button is clicked', async () => {
      const user = userEvent.setup();
      const onViewDetails = vi.fn();
      render(<AccountCard {...defaultProps} onViewDetails={onViewDetails} />);

      const detailsButton = screen.getByTitle('Details');
      await user.click(detailsButton);
      expect(onViewDetails).toHaveBeenCalledTimes(1);
    });

    it('calls onSwitch when switch button is clicked', async () => {
      const user = userEvent.setup();
      const onSwitch = vi.fn();
      render(<AccountCard {...defaultProps} onSwitch={onSwitch} />);

      const switchButton = screen.getByTitle('Switch');
      await user.click(switchButton);
      expect(onSwitch).toHaveBeenCalledTimes(1);
    });

    it('calls onRefresh when refresh button is clicked', async () => {
      const user = userEvent.setup();
      const onRefresh = vi.fn();
      render(<AccountCard {...defaultProps} onRefresh={onRefresh} />);

      const refreshButton = screen.getByTitle('Refresh');
      await user.click(refreshButton);
      expect(onRefresh).toHaveBeenCalledTimes(1);
    });

    it('calls onExport when export button is clicked', async () => {
      const user = userEvent.setup();
      const onExport = vi.fn();
      render(<AccountCard {...defaultProps} onExport={onExport} />);

      const exportButton = screen.getByTitle('Export');
      await user.click(exportButton);
      expect(onExport).toHaveBeenCalledTimes(1);
    });

    it('calls onDelete when delete button is clicked', async () => {
      const user = userEvent.setup();
      const onDelete = vi.fn();
      render(<AccountCard {...defaultProps} onDelete={onDelete} />);

      const deleteButton = screen.getByTitle('Delete');
      await user.click(deleteButton);
      expect(onDelete).toHaveBeenCalledTimes(1);
    });

    it('calls onToggleProxy when proxy toggle is clicked', async () => {
      const user = userEvent.setup();
      const onToggleProxy = vi.fn();
      render(<AccountCard {...defaultProps} onToggleProxy={onToggleProxy} />);

      const toggleButton = screen.getByTitle('Disable Proxy');
      await user.click(toggleButton);
      expect(onToggleProxy).toHaveBeenCalledTimes(1);
    });
  });

  describe('disabled states', () => {
    it('disables switch button when isSwitching is true', () => {
      render(<AccountCard {...defaultProps} isSwitching />);
      const switchButton = screen.getByTitle('Loading...');
      expect(switchButton).toBeDisabled();
    });

    it('disables switch button when account is disabled', () => {
      const disabledAccount = createMockAccount({ disabled: true });
      render(<AccountCard {...defaultProps} account={disabledAccount} />);
      // Multiple elements have the same title, find all and filter to buttons
      const disabledElements = screen.getAllByTitle('This account is disabled');
      const switchButton = disabledElements.find(el => el.tagName === 'BUTTON');
      expect(switchButton).toBeDisabled();
    });

    it('disables refresh button when isRefreshing is true', () => {
      render(<AccountCard {...defaultProps} isRefreshing />);
      const refreshButton = screen.getByTitle('Refresh');
      expect(refreshButton).toBeDisabled();
    });
  });

  describe('quota display', () => {
    it('displays quota percentages for models', () => {
      render(<AccountCard {...defaultProps} />);
      expect(screen.getByText('75%')).toBeInTheDocument();
      expect(screen.getByText('50%')).toBeInTheDocument();
      expect(screen.getByText('25%')).toBeInTheDocument();
      expect(screen.getByText('10%')).toBeInTheDocument();
    });

    it('displays forbidden message when account is forbidden', () => {
      const forbiddenAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          is_forbidden: true,
        },
      });
      render(<AccountCard {...defaultProps} account={forbiddenAccount} />);
      expect(screen.getByText('Account access is forbidden')).toBeInTheDocument();
    });

    it('displays model names', () => {
      render(<AccountCard {...defaultProps} />);
      expect(screen.getByText('G3 Pro')).toBeInTheDocument();
      expect(screen.getByText('G3 Flash')).toBeInTheDocument();
      expect(screen.getByText('G3 Image')).toBeInTheDocument();
      expect(screen.getByText('Claude 4.5')).toBeInTheDocument();
    });
  });

  describe('styling', () => {
    it('applies current account styling when isCurrent is true', () => {
      render(<AccountCard {...defaultProps} isCurrent />);
      const card = screen.getByText('test@example.com').closest('div[class*="rounded-xl"]');
      expect(card).toHaveClass('bg-blue-50/30');
    });

    it('applies opacity when account is disabled', () => {
      const disabledAccount = createMockAccount({ disabled: true });
      render(<AccountCard {...defaultProps} account={disabledAccount} />);
      const card = screen.getByText('test@example.com').closest('div[class*="rounded-xl"]');
      expect(card).toHaveClass('opacity-70');
    });

    it('applies opacity when refreshing', () => {
      render(<AccountCard {...defaultProps} isRefreshing />);
      const card = screen.getByText('test@example.com').closest('div[class*="rounded-xl"]');
      expect(card).toHaveClass('opacity-70');
    });
  });

  describe('proxy toggle state', () => {
    it('shows enable proxy title when proxy is disabled', () => {
      const proxyDisabledAccount = createMockAccount({ proxy_disabled: true });
      render(<AccountCard {...defaultProps} account={proxyDisabledAccount} />);
      expect(screen.getByTitle('Enable Proxy')).toBeInTheDocument();
    });

    it('shows disable proxy title when proxy is enabled', () => {
      render(<AccountCard {...defaultProps} />);
      expect(screen.getByTitle('Disable Proxy')).toBeInTheDocument();
    });
  });
});
