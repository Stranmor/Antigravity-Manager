import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import Toast, { ToastType } from './Toast';

describe('Toast', () => {
  const defaultProps = {
    id: 'test-toast',
    message: 'Test message',
    type: 'info' as ToastType,
    onClose: vi.fn(),
  };

  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  describe('rendering', () => {
    it('renders the message', () => {
      render(<Toast {...defaultProps} />);
      expect(screen.getByText('Test message')).toBeInTheDocument();
    });

    it('renders success icon for success type', () => {
      render(<Toast {...defaultProps} type="success" />);
      expect(screen.getByText('Test message')).toBeInTheDocument();
      // The icon has a specific class for success
      const toast = screen.getByText('Test message').closest('div');
      expect(toast).toHaveClass('border-green-100');
    });

    it('renders error icon for error type', () => {
      render(<Toast {...defaultProps} type="error" />);
      const toast = screen.getByText('Test message').closest('div');
      expect(toast).toHaveClass('border-red-100');
    });

    it('renders warning icon for warning type', () => {
      render(<Toast {...defaultProps} type="warning" />);
      const toast = screen.getByText('Test message').closest('div');
      expect(toast).toHaveClass('border-yellow-100');
    });

    it('renders info icon for info type', () => {
      render(<Toast {...defaultProps} type="info" />);
      const toast = screen.getByText('Test message').closest('div');
      expect(toast).toHaveClass('border-blue-100');
    });
  });

  describe('auto-close behavior', () => {
    it('calls onClose after the default duration', () => {
      const onClose = vi.fn();
      render(<Toast {...defaultProps} onClose={onClose} />);

      // Default duration is 3000ms, plus 300ms for transition
      act(() => {
        vi.advanceTimersByTime(3000);
      });

      // Wait for the transition timeout
      act(() => {
        vi.advanceTimersByTime(300);
      });

      expect(onClose).toHaveBeenCalledWith('test-toast');
    });

    it('calls onClose after custom duration', () => {
      const onClose = vi.fn();
      render(<Toast {...defaultProps} onClose={onClose} duration={5000} />);

      act(() => {
        vi.advanceTimersByTime(5000);
      });

      act(() => {
        vi.advanceTimersByTime(300);
      });

      expect(onClose).toHaveBeenCalledWith('test-toast');
    });

    it('does not auto-close when duration is 0', () => {
      const onClose = vi.fn();
      render(<Toast {...defaultProps} onClose={onClose} duration={0} />);

      act(() => {
        vi.advanceTimersByTime(10000);
      });

      expect(onClose).not.toHaveBeenCalled();
    });

    it('does not auto-close when duration is negative', () => {
      const onClose = vi.fn();
      render(<Toast {...defaultProps} onClose={onClose} duration={-1} />);

      act(() => {
        vi.advanceTimersByTime(10000);
      });

      expect(onClose).not.toHaveBeenCalled();
    });
  });

  describe('manual close', () => {
    it('closes when clicking the close button', async () => {
      vi.useRealTimers();
      const user = userEvent.setup();
      const onClose = vi.fn();
      render(<Toast {...defaultProps} onClose={onClose} duration={0} />);

      const closeButton = screen.getByRole('button');
      await user.click(closeButton);

      await waitFor(() => {
        expect(onClose).toHaveBeenCalledWith('test-toast');
      }, { timeout: 500 });
    });
  });

  describe('visibility animation', () => {
    it('renders toast element', () => {
      vi.useRealTimers();
      render(<Toast {...defaultProps} />);
      // The toast should be rendered
      const toast = screen.getByText('Test message');
      expect(toast).toBeInTheDocument();
    });
  });
});
