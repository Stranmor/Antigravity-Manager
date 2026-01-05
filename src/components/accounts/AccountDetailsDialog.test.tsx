import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import AccountDetailsDialog from './AccountDetailsDialog';
import { Account } from '../../types/account';

// Mock react-dom createPortal to render inline for testing
vi.mock('react-dom', async () => {
  const actual = await vi.importActual('react-dom');
  return {
    ...actual,
    createPortal: (children: React.ReactNode) => children,
  };
});

// Mock react-i18next
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => {
      const translations: Record<string, string> = {
        'accounts.details.title': 'Account Details',
        'accounts.reset_time': 'Reset time',
        'accounts.no_data': 'No data available',
        'common.unknown': 'Unknown',
      };
      return translations[key] || key;
    },
  }),
}));

// Mock format utility
vi.mock('../../utils/format', () => ({
  formatDate: (date: string) => {
    if (!date) return null;
    return '2025-01-16 12:00';
  },
}));

const defaultQuota = {
  models: [
    { name: 'gemini-3-pro-high', percentage: 75, reset_time: '2025-01-16T12:00:00Z' },
    { name: 'gemini-3-flash', percentage: 50, reset_time: '2025-01-16T12:00:00Z' },
    { name: 'claude-sonnet-4-5-thinking', percentage: 60, reset_time: '2025-01-16T12:00:00Z' },
    { name: 'gemini-3-pro-image', percentage: 25, reset_time: '2025-01-16T12:00:00Z' },
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

describe('AccountDetailsDialog', () => {
  const defaultProps = {
    account: createMockAccount(),
    onClose: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('rendering with null account', () => {
    it('returns null when account is null', () => {
      const { container } = render(
        <AccountDetailsDialog account={null} onClose={vi.fn()} />
      );
      expect(container.firstChild).toBeNull();
    });
  });

  describe('rendering with account', () => {
    it('renders dialog title', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      expect(screen.getByText('Account Details')).toBeInTheDocument();
    });

    it('renders account email in header', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      expect(screen.getByText('test@example.com')).toBeInTheDocument();
    });

    it('renders close button', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      // Close button should have X icon
      const closeButtons = screen.getAllByRole('button');
      expect(closeButtons.length).toBeGreaterThan(0);
    });

    it('renders all model quotas', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      expect(screen.getByText('gemini-3-pro-high')).toBeInTheDocument();
      expect(screen.getByText('gemini-3-flash')).toBeInTheDocument();
      expect(screen.getByText('claude-sonnet-4-5-thinking')).toBeInTheDocument();
      expect(screen.getByText('gemini-3-pro-image')).toBeInTheDocument();
    });

    it('renders quota percentages', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      expect(screen.getByText('75%')).toBeInTheDocument();
      expect(screen.getByText('50%')).toBeInTheDocument();
      expect(screen.getByText('60%')).toBeInTheDocument();
      expect(screen.getByText('25%')).toBeInTheDocument();
    });

    it('renders reset times', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      const resetTimes = screen.getAllByText(/Reset time: 2025-01-16 12:00/);
      expect(resetTimes.length).toBe(4);
    });
  });

  describe('close functionality', () => {
    it('calls onClose when close button is clicked', async () => {
      const user = userEvent.setup();
      const onClose = vi.fn();
      render(<AccountDetailsDialog account={createMockAccount()} onClose={onClose} />);

      // Find the close button (first button in the header)
      const closeButton = screen.getAllByRole('button')[0];
      await user.click(closeButton);
      expect(onClose).toHaveBeenCalledTimes(1);
    });

    it('calls onClose when backdrop is clicked', async () => {
      const user = userEvent.setup();
      const onClose = vi.fn();
      render(<AccountDetailsDialog account={createMockAccount()} onClose={onClose} />);

      // Find the backdrop by its class
      const backdrop = document.querySelector('.modal-backdrop');
      if (backdrop) {
        await user.click(backdrop);
        expect(onClose).toHaveBeenCalledTimes(1);
      }
    });
  });

  describe('quota color coding', () => {
    it('applies green styling for quotas >= 50%', () => {
      const highQuotaAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          models: [
            { name: 'high-quota-model', percentage: 75, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });
      render(<AccountDetailsDialog account={highQuotaAccount} onClose={vi.fn()} />);

      const percentage = screen.getByText('75%');
      expect(percentage).toHaveClass('bg-green-50');
    });

    it('applies orange styling for quotas between 20% and 50%', () => {
      const mediumQuotaAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          models: [
            { name: 'medium-quota-model', percentage: 35, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });
      render(<AccountDetailsDialog account={mediumQuotaAccount} onClose={vi.fn()} />);

      const percentage = screen.getByText('35%');
      expect(percentage).toHaveClass('bg-orange-50');
    });

    it('applies red styling for quotas < 20%', () => {
      const lowQuotaAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          models: [
            { name: 'low-quota-model', percentage: 10, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });
      render(<AccountDetailsDialog account={lowQuotaAccount} onClose={vi.fn()} />);

      const percentage = screen.getByText('10%');
      expect(percentage).toHaveClass('bg-red-50');
    });
  });

  describe('progress bars', () => {
    it('renders progress bars for each model', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      // Progress bars have h-1.5 class
      const progressBars = document.querySelectorAll('.h-1\\.5');
      expect(progressBars.length).toBeGreaterThan(0);
    });

    it('sets correct width based on percentage', () => {
      const account = createMockAccount({
        quota: {
          ...defaultQuota,
          models: [
            { name: 'test-model', percentage: 42, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });
      render(<AccountDetailsDialog account={account} onClose={vi.fn()} />);

      const progressBar = document.querySelector('[style*="width: 42%"]');
      expect(progressBar).toBeInTheDocument();
    });
  });

  describe('empty state', () => {
    it('renders empty grid when models array is empty', () => {
      const emptyModelsAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          models: [],
        },
      });
      render(<AccountDetailsDialog account={emptyModelsAccount} onClose={vi.fn()} />);
      // With empty array, grid is rendered but empty (no model cards)
      const grid = document.querySelector('.grid');
      expect(grid).toBeInTheDocument();
      expect(grid?.children.length).toBe(0);
    });

    it('renders no data message when quota is undefined', () => {
      const noQuotaAccount = createMockAccount({ quota: undefined });
      render(<AccountDetailsDialog account={noQuotaAccount} onClose={vi.fn()} />);
      expect(screen.getByText('No data available')).toBeInTheDocument();
    });
  });

  describe('unknown reset time', () => {
    it('shows Unknown when reset_time is empty', () => {
      const noResetTimeAccount = createMockAccount({
        quota: {
          ...defaultQuota,
          models: [
            { name: 'no-reset-model', percentage: 50, reset_time: '' },
          ],
        },
      });
      render(<AccountDetailsDialog account={noResetTimeAccount} onClose={vi.fn()} />);
      expect(screen.getByText(/Unknown/)).toBeInTheDocument();
    });
  });

  describe('modal structure', () => {
    it('renders with modal-open class', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      const modal = document.querySelector('.modal-open');
      expect(modal).toBeInTheDocument();
    });

    it('renders modal-box for content', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      const modalBox = document.querySelector('.modal-box');
      expect(modalBox).toBeInTheDocument();
    });

    it('renders modal-backdrop', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      const backdrop = document.querySelector('.modal-backdrop');
      expect(backdrop).toBeInTheDocument();
    });

    it('has draggable region at top', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      const dragRegion = document.querySelector('[data-tauri-drag-region]');
      expect(dragRegion).toBeInTheDocument();
    });
  });

  describe('grid layout', () => {
    it('renders models in grid layout', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      const grid = document.querySelector('.grid');
      expect(grid).toBeInTheDocument();
    });

    it('uses 2-column grid on medium screens', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      const grid = document.querySelector('.md\\:grid-cols-2');
      expect(grid).toBeInTheDocument();
    });
  });

  describe('hover effects', () => {
    it('model cards have hover transition classes', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      const modelCard = screen.getByText('gemini-3-pro-high').closest('.rounded-xl');
      expect(modelCard).toHaveClass('transition-all');
    });
  });

  describe('accessibility', () => {
    it('has accessible button for closing', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      const buttons = screen.getAllByRole('button');
      expect(buttons.length).toBeGreaterThan(0);
    });

    it('email is displayed as monospace text', () => {
      render(<AccountDetailsDialog {...defaultProps} />);
      const emailElement = screen.getByText('test@example.com');
      expect(emailElement).toHaveClass('font-mono');
    });
  });

  describe('multiple accounts', () => {
    it('updates display when account prop changes', async () => {
      const { rerender } = render(<AccountDetailsDialog {...defaultProps} />);
      expect(screen.getByText('test@example.com')).toBeInTheDocument();

      const newAccount = createMockAccount({
        email: 'new@example.com',
        quota: {
          ...defaultQuota,
          models: [
            { name: 'new-model', percentage: 90, reset_time: '2025-01-16T12:00:00Z' },
          ],
        },
      });

      rerender(<AccountDetailsDialog account={newAccount} onClose={vi.fn()} />);

      await waitFor(() => {
        expect(screen.getByText('new@example.com')).toBeInTheDocument();
        expect(screen.getByText('new-model')).toBeInTheDocument();
        expect(screen.getByText('90%')).toBeInTheDocument();
      });
    });
  });
});
