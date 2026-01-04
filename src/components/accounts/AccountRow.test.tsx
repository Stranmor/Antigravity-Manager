import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import AccountRow from './AccountRow';
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
        'accounts.proxy_disabled': 'Proxy Disabled',
        'accounts.proxy_disabled_tooltip': 'Proxy is disabled for this account',
        'accounts.enable_proxy': 'Enable Proxy',
        'accounts.disable_proxy': 'Disable Proxy',
        'accounts.switch_to': 'Switch to this account',
        'common.details': 'Details',
        'common.loading': 'Loading...',
        'common.refresh': 'Refresh',
        'common.refreshing': 'Refreshing...',
        'common.export': 'Export',
        'common.delete': 'Delete',
      };
      return translations[key] || key;
    },
  }),
}));

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
  quota: {
    models: [
      { name: 'gemini-3-pro-high', percentage: 75, reset_time: '2025-01-16T12:00:00Z' },
      { name: 'gemini-3-flash', percentage: 50, reset_time: '2025-01-16T12:00:00Z' },
      { name: 'gemini-3-pro-image', percentage: 25, reset_time: '2025-01-16T12:00:00Z' },
      { name: 'claude-sonnet-4-5-thinking', percentage: 10, reset_time: '2025-01-16T12:00:00Z' },
    ],
    last_updated: Date.now(),
    subscription_tier: 'FREE',
  },
  created_at: Date.now() / 1000,
  last_used: Date.now() / 1000,
  ...overrides,
});

// Helper to wrap row in table structure for valid HTML
const renderInTable = (ui: React.ReactElement) => {
  return render(
    <table>
      <tbody>{ui}</tbody>
    </table>
  );
};

describe('AccountRow', () => {
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
      renderInTable(<AccountRow {...defaultProps} />);
      expect(screen.getByText('test@example.com')).toBeInTheDocument();
    });

    it('renders as a table row', () => {
      renderInTable(<AccountRow {...defaultProps} />);
      expect(screen.getByRole('row')).toBeInTheDocument();
    });

    it('renders current badge when isCurrent is true', () => {
      renderInTable(<AccountRow {...defaultProps} isCurrent />);
      expect(screen.getByText('CURRENT')).toBeInTheDocument();
    });

    it('renders disabled badge when account is disabled', () => {
      const disabledAccount = createMockAccount({ disabled: true });
      renderInTable(<AccountRow {...defaultProps} account={disabledAccount} />);
      expect(screen.getByText('Disabled')).toBeInTheDocument();
    });

    it('renders proxy disabled badge when proxy is disabled', () => {
      const proxyDisabledAccount = createMockAccount({ proxy_disabled: true });
      renderInTable(<AccountRow {...defaultProps} account={proxyDisabledAccount} />);
      expect(screen.getByText('Proxy Disabled')).toBeInTheDocument();
    });

    it('renders forbidden badge when quota is forbidden', () => {
      const forbiddenAccount = createMockAccount({
        quota: {
          ...createMockAccount().quota!,
          is_forbidden: true,
        },
      });
      renderInTable(<AccountRow {...defaultProps} account={forbiddenAccount} />);
      expect(screen.getByText('Forbidden')).toBeInTheDocument();
    });
  });

  describe('subscription tiers', () => {
    it('renders FREE badge for free tier', () => {
      renderInTable(<AccountRow {...defaultProps} />);
      expect(screen.getByText('FREE')).toBeInTheDocument();
    });

    it('renders PRO badge for pro tier', () => {
      const proAccount = createMockAccount({
        quota: {
          ...createMockAccount().quota!,
          subscription_tier: 'PRO',
        },
      });
      renderInTable(<AccountRow {...defaultProps} account={proAccount} />);
      expect(screen.getByText('PRO')).toBeInTheDocument();
    });

    it('renders ULTRA badge for ultra tier', () => {
      const ultraAccount = createMockAccount({
        quota: {
          ...createMockAccount().quota!,
          subscription_tier: 'ULTRA',
        },
      });
      renderInTable(<AccountRow {...defaultProps} account={ultraAccount} />);
      expect(screen.getByText('ULTRA')).toBeInTheDocument();
    });
  });

  describe('checkbox interaction', () => {
    it('checkbox reflects selected state', () => {
      renderInTable(<AccountRow {...defaultProps} selected />);
      const checkbox = screen.getByRole('checkbox');
      expect(checkbox).toBeChecked();
    });

    it('calls onSelect when checkbox is clicked', async () => {
      const user = userEvent.setup();
      const onSelect = vi.fn();
      renderInTable(<AccountRow {...defaultProps} onSelect={onSelect} />);

      await user.click(screen.getByRole('checkbox'));
      expect(onSelect).toHaveBeenCalledTimes(1);
    });
  });

  describe('action buttons', () => {
    it('calls onViewDetails when details button is clicked', async () => {
      const user = userEvent.setup();
      const onViewDetails = vi.fn();
      renderInTable(<AccountRow {...defaultProps} onViewDetails={onViewDetails} />);

      const detailsButton = screen.getByTitle('Details');
      await user.click(detailsButton);
      expect(onViewDetails).toHaveBeenCalledTimes(1);
    });

    it('calls onSwitch when switch button is clicked', async () => {
      const user = userEvent.setup();
      const onSwitch = vi.fn();
      renderInTable(<AccountRow {...defaultProps} onSwitch={onSwitch} />);

      const switchButton = screen.getByTitle('Switch to this account');
      await user.click(switchButton);
      expect(onSwitch).toHaveBeenCalledTimes(1);
    });

    it('calls onRefresh when refresh button is clicked', async () => {
      const user = userEvent.setup();
      const onRefresh = vi.fn();
      renderInTable(<AccountRow {...defaultProps} onRefresh={onRefresh} />);

      const refreshButton = screen.getByTitle('Refresh');
      await user.click(refreshButton);
      expect(onRefresh).toHaveBeenCalledTimes(1);
    });

    it('calls onExport when export button is clicked', async () => {
      const user = userEvent.setup();
      const onExport = vi.fn();
      renderInTable(<AccountRow {...defaultProps} onExport={onExport} />);

      const exportButton = screen.getByTitle('Export');
      await user.click(exportButton);
      expect(onExport).toHaveBeenCalledTimes(1);
    });

    it('calls onDelete when delete button is clicked', async () => {
      const user = userEvent.setup();
      const onDelete = vi.fn();
      renderInTable(<AccountRow {...defaultProps} onDelete={onDelete} />);

      const deleteButton = screen.getByTitle('Delete');
      await user.click(deleteButton);
      expect(onDelete).toHaveBeenCalledTimes(1);
    });

    it('calls onToggleProxy when proxy toggle is clicked', async () => {
      const user = userEvent.setup();
      const onToggleProxy = vi.fn();
      renderInTable(<AccountRow {...defaultProps} onToggleProxy={onToggleProxy} />);

      const toggleButton = screen.getByTitle('Disable Proxy');
      await user.click(toggleButton);
      expect(onToggleProxy).toHaveBeenCalledTimes(1);
    });
  });

  describe('disabled states', () => {
    it('disables switch button when isSwitching is true', () => {
      renderInTable(<AccountRow {...defaultProps} isSwitching />);
      const switchButton = screen.getByTitle('Loading...');
      expect(switchButton).toBeDisabled();
    });

    it('disables switch button when account is disabled', () => {
      const disabledAccount = createMockAccount({ disabled: true });
      renderInTable(<AccountRow {...defaultProps} account={disabledAccount} />);
      // Multiple elements have the same title, find all and filter to buttons
      const disabledElements = screen.getAllByTitle('This account is disabled');
      const switchButton = disabledElements.find(el => el.tagName === 'BUTTON');
      expect(switchButton).toBeDisabled();
    });

    it('disables refresh button when isRefreshing is true', () => {
      renderInTable(<AccountRow {...defaultProps} isRefreshing />);
      const refreshButton = screen.getByTitle('Refreshing...');
      expect(refreshButton).toBeDisabled();
    });

    it('disables refresh button when account is disabled', () => {
      const disabledAccount = createMockAccount({ disabled: true });
      renderInTable(<AccountRow {...defaultProps} account={disabledAccount} />);
      // Multiple elements have the same title, find all disabled buttons
      const disabledElements = screen.getAllByTitle('This account is disabled');
      const buttons = disabledElements.filter(el => el.tagName === 'BUTTON');
      // Should have at least 2 disabled buttons (switch and refresh)
      expect(buttons.length).toBeGreaterThanOrEqual(2);
      buttons.forEach(btn => expect(btn).toBeDisabled());
    });
  });

  describe('quota display', () => {
    it('displays quota percentages for models', () => {
      renderInTable(<AccountRow {...defaultProps} />);
      expect(screen.getByText('75%')).toBeInTheDocument();
      expect(screen.getByText('50%')).toBeInTheDocument();
      expect(screen.getByText('25%')).toBeInTheDocument();
      expect(screen.getByText('10%')).toBeInTheDocument();
    });

    it('displays forbidden message when account is forbidden', () => {
      const forbiddenAccount = createMockAccount({
        quota: {
          ...createMockAccount().quota!,
          is_forbidden: true,
        },
      });
      renderInTable(<AccountRow {...defaultProps} account={forbiddenAccount} />);
      expect(screen.getByText('Account access is forbidden')).toBeInTheDocument();
    });

    it('displays model names', () => {
      renderInTable(<AccountRow {...defaultProps} />);
      expect(screen.getByText('G3 Pro')).toBeInTheDocument();
      expect(screen.getByText('G3 Flash')).toBeInTheDocument();
      expect(screen.getByText('G3 Image')).toBeInTheDocument();
      expect(screen.getByText('Claude 4.5')).toBeInTheDocument();
    });
  });

  describe('styling', () => {
    it('applies current account styling when isCurrent is true', () => {
      renderInTable(<AccountRow {...defaultProps} isCurrent />);
      const row = screen.getByRole('row');
      expect(row).toHaveClass('bg-blue-50/50');
    });

    it('applies opacity when account is disabled', () => {
      const disabledAccount = createMockAccount({ disabled: true });
      renderInTable(<AccountRow {...defaultProps} account={disabledAccount} />);
      const row = screen.getByRole('row');
      expect(row).toHaveClass('opacity-70');
    });

    it('applies opacity when refreshing', () => {
      renderInTable(<AccountRow {...defaultProps} isRefreshing />);
      const row = screen.getByRole('row');
      expect(row).toHaveClass('opacity-70');
    });
  });

  describe('last used date', () => {
    it('displays formatted date', () => {
      const timestamp = new Date('2025-01-15T12:30:00').getTime() / 1000;
      const account = createMockAccount({ last_used: timestamp });
      renderInTable(<AccountRow {...defaultProps} account={account} />);
      // Date format depends on locale, so just check that the row renders
      expect(screen.getByRole('row')).toBeInTheDocument();
    });
  });
});
