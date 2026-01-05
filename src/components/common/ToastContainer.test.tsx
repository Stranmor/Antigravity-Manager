import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { render, screen, waitFor, act } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import ToastContainer, { showToast } from './ToastContainer';

describe('ToastContainer', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  describe('rendering', () => {
    it('renders without crashing', () => {
      render(<ToastContainer />);
      // The container renders but is empty initially
      expect(document.body.querySelector('.fixed')).toBeInTheDocument();
    });

    it('renders container with correct positioning classes', () => {
      render(<ToastContainer />);
      const container = document.body.querySelector('.fixed');
      expect(container).toHaveClass('top-24');
      expect(container).toHaveClass('right-8');
      expect(container).toHaveClass('z-[200]');
    });

    it('renders as a portal in document.body', () => {
      const { container } = render(<ToastContainer />);
      // Component renders empty in the mounting point
      expect(container).toBeEmptyDOMElement();
      // But the portal content is in body
      expect(document.body.querySelector('.fixed')).toBeInTheDocument();
    });
  });

  describe('showToast function', () => {
    it('logs warning when container is not mounted', () => {
      const consoleSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
      showToast('Test message', 'info');
      expect(consoleSpy).toHaveBeenCalledWith('ToastContainer not mounted');
      consoleSpy.mockRestore();
    });

    it('adds toast when container is mounted', () => {
      render(<ToastContainer />);
      act(() => {
        showToast('Hello World', 'success');
      });
      expect(screen.getByText('Hello World')).toBeInTheDocument();
    });

    it('adds multiple toasts', () => {
      render(<ToastContainer />);
      act(() => {
        showToast('First toast', 'info');
        showToast('Second toast', 'success');
        showToast('Third toast', 'error');
      });

      expect(screen.getByText('First toast')).toBeInTheDocument();
      expect(screen.getByText('Second toast')).toBeInTheDocument();
      expect(screen.getByText('Third toast')).toBeInTheDocument();
    });

    it('uses info type by default', () => {
      render(<ToastContainer />);
      act(() => {
        showToast('Default type toast');
      });

      const toast = screen.getByText('Default type toast').closest('div');
      expect(toast).toHaveClass('border-blue-100');
    });

    it('applies correct styling for success type', () => {
      render(<ToastContainer />);
      act(() => {
        showToast('Success message', 'success');
      });

      const toast = screen.getByText('Success message').closest('div');
      expect(toast).toHaveClass('border-green-100');
    });

    it('applies correct styling for error type', () => {
      render(<ToastContainer />);
      act(() => {
        showToast('Error message', 'error');
      });

      const toast = screen.getByText('Error message').closest('div');
      expect(toast).toHaveClass('border-red-100');
    });

    it('applies correct styling for warning type', () => {
      render(<ToastContainer />);
      act(() => {
        showToast('Warning message', 'warning');
      });

      const toast = screen.getByText('Warning message').closest('div');
      expect(toast).toHaveClass('border-yellow-100');
    });
  });

  describe('toast auto-removal', () => {
    it('removes toast after default duration', () => {
      render(<ToastContainer />);
      act(() => {
        showToast('Auto remove toast', 'info');
      });

      expect(screen.getByText('Auto remove toast')).toBeInTheDocument();

      // Default duration is 3000ms + 300ms transition
      act(() => {
        vi.advanceTimersByTime(3000);
      });
      act(() => {
        vi.advanceTimersByTime(300);
      });

      expect(screen.queryByText('Auto remove toast')).not.toBeInTheDocument();
    });

    it('removes toast after custom duration', () => {
      render(<ToastContainer />);
      act(() => {
        showToast('Custom duration toast', 'info', 5000);
      });

      expect(screen.getByText('Custom duration toast')).toBeInTheDocument();

      // Should still be there before 5000ms
      act(() => {
        vi.advanceTimersByTime(4000);
      });
      expect(screen.getByText('Custom duration toast')).toBeInTheDocument();

      // Should be removed after 5000ms + 300ms transition
      act(() => {
        vi.advanceTimersByTime(1300);
      });

      expect(screen.queryByText('Custom duration toast')).not.toBeInTheDocument();
    });

    it('persists toast with duration 0', () => {
      render(<ToastContainer />);
      act(() => {
        showToast('Persistent toast', 'info', 0);
      });

      expect(screen.getByText('Persistent toast')).toBeInTheDocument();

      act(() => {
        vi.advanceTimersByTime(10000);
      });

      expect(screen.getByText('Persistent toast')).toBeInTheDocument();
    });
  });

  describe('manual toast removal', () => {
    it('removes toast when close button is clicked', async () => {
      vi.useRealTimers();
      const user = userEvent.setup();
      render(<ToastContainer />);

      act(() => {
        showToast('Closeable toast', 'info', 0);
      });

      expect(screen.getByText('Closeable toast')).toBeInTheDocument();

      const closeButton = screen.getByRole('button');
      await user.click(closeButton);

      await waitFor(() => {
        expect(screen.queryByText('Closeable toast')).not.toBeInTheDocument();
      }, { timeout: 500 });
    });
  });

  describe('unique toast ids', () => {
    it('generates unique ids for each toast', () => {
      render(<ToastContainer />);
      act(() => {
        showToast('Toast 1', 'info');
        showToast('Toast 2', 'info');
        showToast('Toast 3', 'info');
      });

      // All three toasts should be visible (not replacing each other)
      expect(screen.getByText('Toast 1')).toBeInTheDocument();
      expect(screen.getByText('Toast 2')).toBeInTheDocument();
      expect(screen.getByText('Toast 3')).toBeInTheDocument();
    });
  });

  describe('cleanup on unmount', () => {
    it('clears external reference on unmount', () => {
      const { unmount } = render(<ToastContainer />);

      // showToast should work while mounted
      act(() => {
        showToast('Before unmount', 'info');
      });
      expect(screen.getByText('Before unmount')).toBeInTheDocument();

      unmount();

      // After unmount, showToast should log warning
      const consoleSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});
      showToast('After unmount', 'info');
      expect(consoleSpy).toHaveBeenCalledWith('ToastContainer not mounted');
      consoleSpy.mockRestore();
    });
  });

  describe('toast stacking', () => {
    it('stacks toasts in order', () => {
      render(<ToastContainer />);
      act(() => {
        showToast('First', 'info');
        showToast('Second', 'success');
        showToast('Third', 'error');
      });

      const toasts = screen.getAllByRole('button');
      // Each toast has a close button, so we should have 3 buttons
      expect(toasts).toHaveLength(3);
    });
  });
});
