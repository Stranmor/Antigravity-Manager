import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import AccountGrid from './AccountGrid';
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

describe('AccountGrid', () => {
  const defaultProps = {
    accounts: [createMockAccount()],
    selectedIds: new Set<string>(),
    refreshingIds: new Set<string>(),
    onToggleSelect: vi.fn(),
    currentAccountId: null,
    switchingAccountId: null,
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

  describe('empty state', () => {
    it('renders empty state message when no accounts', () => {
      render(<AccountGrid {...defaultProps} accounts={[]} />);
      expect(screen.getByText('暂无账号')).toBeInTheDocument();
    });

    it('renders helper text in empty state', () => {
      render(<AccountGrid {...defaultProps} accounts={[]} />);
      expect(screen.getByText('点击上方"添加账号"按钮添加第一个账号')).toBeInTheDocument();
    });

    it('does not render grid when no accounts', () => {
      const { container } = render(<AccountGrid {...defaultProps} accounts={[]} />);
      expect(container.querySelector('.grid')).not.toBeInTheDocument();
    });
  });

  describe('rendering accounts', () => {
    it('renders a single account card', () => {
      render(<AccountGrid {...defaultProps} />);
      expect(screen.getByText('test@example.com')).toBeInTheDocument();
    });

    it('renders multiple account cards', () => {
      const accounts = [
        createMockAccount({ id: '1', email: 'user1@example.com' }),
        createMockAccount({ id: '2', email: 'user2@example.com' }),
        createMockAccount({ id: '3', email: 'user3@example.com' }),
      ];
      render(<AccountGrid {...defaultProps} accounts={accounts} />);

      expect(screen.getByText('user1@example.com')).toBeInTheDocument();
      expect(screen.getByText('user2@example.com')).toBeInTheDocument();
      expect(screen.getByText('user3@example.com')).toBeInTheDocument();
    });

    it('renders grid container with correct classes', () => {
      const { container } = render(<AccountGrid {...defaultProps} />);
      const grid = container.querySelector('.grid');
      expect(grid).toHaveClass('grid-cols-1');
      expect(grid).toHaveClass('md:grid-cols-2');
      expect(grid).toHaveClass('lg:grid-cols-3');
      expect(grid).toHaveClass('xl:grid-cols-4');
    });
  });

  describe('selection state', () => {
    it('passes selected state to AccountCard when id is in selectedIds', () => {
      const selectedIds = new Set(['test-id-1']);
      render(<AccountGrid {...defaultProps} selectedIds={selectedIds} />);
      const checkbox = screen.getByRole('checkbox');
      expect(checkbox).toBeChecked();
    });

    it('passes non-selected state to AccountCard when id is not in selectedIds', () => {
      render(<AccountGrid {...defaultProps} />);
      const checkbox = screen.getByRole('checkbox');
      expect(checkbox).not.toBeChecked();
    });
  });

  describe('refreshing state', () => {
    it('passes refreshing state to AccountCard when id is in refreshingIds', () => {
      const refreshingIds = new Set(['test-id-1']);
      render(<AccountGrid {...defaultProps} refreshingIds={refreshingIds} />);
      const refreshButton = screen.getByTitle('Refresh');
      expect(refreshButton).toBeDisabled();
    });

    it('passes non-refreshing state to AccountCard when id is not in refreshingIds', () => {
      render(<AccountGrid {...defaultProps} />);
      const refreshButton = screen.getByTitle('Refresh');
      expect(refreshButton).not.toBeDisabled();
    });
  });

  describe('current account state', () => {
    it('marks account as current when currentAccountId matches', () => {
      render(<AccountGrid {...defaultProps} currentAccountId="test-id-1" />);
      expect(screen.getByText('CURRENT')).toBeInTheDocument();
    });

    it('does not mark account as current when currentAccountId differs', () => {
      render(<AccountGrid {...defaultProps} currentAccountId="other-id" />);
      expect(screen.queryByText('CURRENT')).not.toBeInTheDocument();
    });
  });

  describe('switching account state', () => {
    it('marks account as switching when switchingAccountId matches', () => {
      render(<AccountGrid {...defaultProps} switchingAccountId="test-id-1" />);
      const switchButton = screen.getByTitle('Loading...');
      expect(switchButton).toBeDisabled();
    });

    it('does not disable switch when switchingAccountId differs', () => {
      render(<AccountGrid {...defaultProps} switchingAccountId="other-id" />);
      const switchButton = screen.getByTitle('Switch');
      expect(switchButton).not.toBeDisabled();
    });
  });

  describe('callback propagation', () => {
    it('calls onToggleSelect with account id when checkbox is clicked', async () => {
      const user = userEvent.setup();
      const onToggleSelect = vi.fn();
      render(<AccountGrid {...defaultProps} onToggleSelect={onToggleSelect} />);

      await user.click(screen.getByRole('checkbox'));
      expect(onToggleSelect).toHaveBeenCalledWith('test-id-1');
    });

    it('calls onSwitch with account id when switch button is clicked', async () => {
      const user = userEvent.setup();
      const onSwitch = vi.fn();
      render(<AccountGrid {...defaultProps} onSwitch={onSwitch} />);

      await user.click(screen.getByTitle('Switch'));
      expect(onSwitch).toHaveBeenCalledWith('test-id-1');
    });

    it('calls onRefresh with account id when refresh button is clicked', async () => {
      const user = userEvent.setup();
      const onRefresh = vi.fn();
      render(<AccountGrid {...defaultProps} onRefresh={onRefresh} />);

      await user.click(screen.getByTitle('Refresh'));
      expect(onRefresh).toHaveBeenCalledWith('test-id-1');
    });

    it('calls onViewDetails with account id when details button is clicked', async () => {
      const user = userEvent.setup();
      const onViewDetails = vi.fn();
      render(<AccountGrid {...defaultProps} onViewDetails={onViewDetails} />);

      await user.click(screen.getByTitle('Details'));
      expect(onViewDetails).toHaveBeenCalledWith('test-id-1');
    });

    it('calls onExport with account id when export button is clicked', async () => {
      const user = userEvent.setup();
      const onExport = vi.fn();
      render(<AccountGrid {...defaultProps} onExport={onExport} />);

      await user.click(screen.getByTitle('Export'));
      expect(onExport).toHaveBeenCalledWith('test-id-1');
    });

    it('calls onDelete with account id when delete button is clicked', async () => {
      const user = userEvent.setup();
      const onDelete = vi.fn();
      render(<AccountGrid {...defaultProps} onDelete={onDelete} />);

      await user.click(screen.getByTitle('Delete'));
      expect(onDelete).toHaveBeenCalledWith('test-id-1');
    });

    it('calls onToggleProxy with account id when proxy toggle is clicked', async () => {
      const user = userEvent.setup();
      const onToggleProxy = vi.fn();
      render(<AccountGrid {...defaultProps} onToggleProxy={onToggleProxy} />);

      await user.click(screen.getByTitle('Disable Proxy'));
      expect(onToggleProxy).toHaveBeenCalledWith('test-id-1');
    });
  });

  describe('multiple accounts selection', () => {
    it('shows correct selection state for multiple accounts', () => {
      const accounts = [
        createMockAccount({ id: '1', email: 'user1@example.com' }),
        createMockAccount({ id: '2', email: 'user2@example.com' }),
        createMockAccount({ id: '3', email: 'user3@example.com' }),
      ];
      const selectedIds = new Set(['1', '3']);
      render(<AccountGrid {...defaultProps} accounts={accounts} selectedIds={selectedIds} />);

      const checkboxes = screen.getAllByRole('checkbox');
      expect(checkboxes[0]).toBeChecked();
      expect(checkboxes[1]).not.toBeChecked();
      expect(checkboxes[2]).toBeChecked();
    });
  });
});
