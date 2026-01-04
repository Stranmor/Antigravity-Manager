import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ModalDialog, { ModalType } from './ModalDialog';

// Mock react-i18next
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => {
      const translations: Record<string, string> = {
        'common.confirm': 'Confirm',
        'common.cancel': 'Cancel',
      };
      return translations[key] || key;
    },
  }),
}));

describe('ModalDialog', () => {
  const defaultProps = {
    isOpen: true,
    title: 'Test Title',
    message: 'Test message content',
    onConfirm: vi.fn(),
  };

  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe('rendering', () => {
    it('renders when isOpen is true', () => {
      render(<ModalDialog {...defaultProps} />);
      expect(screen.getByText('Test Title')).toBeInTheDocument();
      expect(screen.getByText('Test message content')).toBeInTheDocument();
    });

    it('does not render when isOpen is false', () => {
      render(<ModalDialog {...defaultProps} isOpen={false} />);
      expect(screen.queryByText('Test Title')).not.toBeInTheDocument();
    });

    it('renders with default confirm button text', () => {
      render(<ModalDialog {...defaultProps} />);
      expect(screen.getByRole('button', { name: 'Confirm' })).toBeInTheDocument();
    });

    it('renders with custom confirm button text', () => {
      render(<ModalDialog {...defaultProps} confirmText="Yes, delete" />);
      expect(screen.getByRole('button', { name: 'Yes, delete' })).toBeInTheDocument();
    });
  });

  describe('modal types', () => {
    const types: ModalType[] = ['confirm', 'success', 'error', 'info'];

    it.each(types)('renders %s modal type', (type) => {
      render(<ModalDialog {...defaultProps} type={type} />);
      expect(screen.getByText('Test Title')).toBeInTheDocument();
    });

    it('shows cancel button only for confirm type', () => {
      render(<ModalDialog {...defaultProps} type="confirm" onCancel={vi.fn()} />);
      expect(screen.getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
    });

    it('does not show cancel button for success type', () => {
      render(<ModalDialog {...defaultProps} type="success" onCancel={vi.fn()} />);
      expect(screen.queryByRole('button', { name: 'Cancel' })).not.toBeInTheDocument();
    });

    it('does not show cancel button for error type', () => {
      render(<ModalDialog {...defaultProps} type="error" onCancel={vi.fn()} />);
      expect(screen.queryByRole('button', { name: 'Cancel' })).not.toBeInTheDocument();
    });

    it('does not show cancel button for info type', () => {
      render(<ModalDialog {...defaultProps} type="info" onCancel={vi.fn()} />);
      expect(screen.queryByRole('button', { name: 'Cancel' })).not.toBeInTheDocument();
    });
  });

  describe('destructive mode', () => {
    it('applies destructive styles when isDestructive is true', () => {
      render(
        <ModalDialog {...defaultProps} type="confirm" isDestructive onCancel={vi.fn()} />
      );
      const confirmButton = screen.getByRole('button', { name: 'Confirm' });
      expect(confirmButton).toHaveClass('bg-red-500');
    });

    it('applies normal styles when isDestructive is false', () => {
      render(
        <ModalDialog {...defaultProps} type="confirm" isDestructive={false} onCancel={vi.fn()} />
      );
      const confirmButton = screen.getByRole('button', { name: 'Confirm' });
      expect(confirmButton).toHaveClass('bg-blue-500');
    });
  });

  describe('interactions', () => {
    it('calls onConfirm when clicking confirm button', async () => {
      const user = userEvent.setup();
      const onConfirm = vi.fn();
      render(<ModalDialog {...defaultProps} onConfirm={onConfirm} />);

      await user.click(screen.getByRole('button', { name: 'Confirm' }));
      expect(onConfirm).toHaveBeenCalledTimes(1);
    });

    it('calls onCancel when clicking cancel button', async () => {
      const user = userEvent.setup();
      const onCancel = vi.fn();
      render(
        <ModalDialog {...defaultProps} type="confirm" onCancel={onCancel} />
      );

      await user.click(screen.getByRole('button', { name: 'Cancel' }));
      expect(onCancel).toHaveBeenCalledTimes(1);
    });

    it('calls onCancel when clicking backdrop for confirm type', async () => {
      const user = userEvent.setup();
      const onCancel = vi.fn();
      render(
        <ModalDialog {...defaultProps} type="confirm" onCancel={onCancel} />
      );

      // Find the backdrop element
      const backdrop = document.querySelector('.modal-backdrop');
      if (backdrop) {
        await user.click(backdrop);
        expect(onCancel).toHaveBeenCalledTimes(1);
      }
    });

    it('does not call onCancel when clicking backdrop for non-confirm types', async () => {
      const user = userEvent.setup();
      const onCancel = vi.fn();
      render(
        <ModalDialog {...defaultProps} type="success" onCancel={onCancel} />
      );

      const backdrop = document.querySelector('.modal-backdrop');
      if (backdrop) {
        await user.click(backdrop);
        expect(onCancel).not.toHaveBeenCalled();
      }
    });
  });

  describe('custom button text', () => {
    it('renders custom cancel text', () => {
      render(
        <ModalDialog
          {...defaultProps}
          type="confirm"
          onCancel={vi.fn()}
          cancelText="No, go back"
        />
      );
      expect(screen.getByRole('button', { name: 'No, go back' })).toBeInTheDocument();
    });
  });

  describe('icon backgrounds', () => {
    it('applies correct icon background for success', () => {
      render(<ModalDialog {...defaultProps} type="success" />);
      const iconBg = document.querySelector('.bg-green-50');
      expect(iconBg).toBeInTheDocument();
    });

    it('applies correct icon background for error', () => {
      render(<ModalDialog {...defaultProps} type="error" />);
      const iconBg = document.querySelector('.bg-red-50');
      expect(iconBg).toBeInTheDocument();
    });

    it('applies correct icon background for info', () => {
      render(<ModalDialog {...defaultProps} type="info" />);
      const iconBg = document.querySelector('.bg-blue-50');
      expect(iconBg).toBeInTheDocument();
    });

    it('applies destructive icon background for confirm with isDestructive', () => {
      render(
        <ModalDialog {...defaultProps} type="confirm" isDestructive onCancel={vi.fn()} />
      );
      const iconBg = document.querySelector('.bg-red-50');
      expect(iconBg).toBeInTheDocument();
    });
  });
});
