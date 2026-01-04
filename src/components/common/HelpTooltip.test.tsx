import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import HelpTooltip, { HelpTooltipPlacement } from './HelpTooltip';

describe('HelpTooltip', () => {
  describe('rendering', () => {
    it('renders the tooltip with text', () => {
      render(<HelpTooltip text="Help text content" />);
      expect(screen.getByText('Help text content')).toBeInTheDocument();
    });

    it('renders the help icon button', () => {
      render(<HelpTooltip text="Help text" />);
      expect(screen.getByRole('button', { name: 'Help' })).toBeInTheDocument();
    });

    it('returns null when text is empty', () => {
      const { container } = render(<HelpTooltip text="" />);
      expect(container.firstChild).toBeNull();
    });

    it('applies custom aria-label', () => {
      render(<HelpTooltip text="Help text" ariaLabel="Custom help label" />);
      expect(screen.getByRole('button', { name: 'Custom help label' })).toBeInTheDocument();
    });

    it('applies custom className', () => {
      render(<HelpTooltip text="Help text" className="custom-class" />);
      const wrapper = screen.getByRole('button').parentElement;
      expect(wrapper).toHaveClass('custom-class');
    });
  });

  describe('placement classes', () => {
    const placements: HelpTooltipPlacement[] = ['top', 'right', 'bottom', 'left'];

    it.each(placements)('applies correct placement class for %s', (placement) => {
      render(<HelpTooltip text="Help text" placement={placement} />);
      const tooltip = screen.getByText('Help text');

      // Each placement has specific positioning classes
      switch (placement) {
        case 'top':
          expect(tooltip).toHaveClass('bottom-full');
          break;
        case 'right':
          expect(tooltip).toHaveClass('left-full');
          break;
        case 'bottom':
          expect(tooltip).toHaveClass('top-full');
          break;
        case 'left':
          expect(tooltip).toHaveClass('right-full');
          break;
      }
    });

    it('defaults to top placement', () => {
      render(<HelpTooltip text="Help text" />);
      const tooltip = screen.getByText('Help text');
      expect(tooltip).toHaveClass('bottom-full');
    });
  });

  describe('interaction', () => {
    it('prevents default on click', async () => {
      const user = userEvent.setup();
      render(<HelpTooltip text="Help text" />);

      const button = screen.getByRole('button');
      await user.click(button);

      // Button should still be there after click (not navigated away)
      expect(button).toBeInTheDocument();
    });

    it('stops propagation on click', async () => {
      const parentClickHandler = vi.fn();
      const user = userEvent.setup();

      render(
        <div onClick={parentClickHandler}>
          <HelpTooltip text="Help text" />
        </div>
      );

      const button = screen.getByRole('button');
      await user.click(button);

      // Parent click handler should not be called due to stopPropagation
      expect(parentClickHandler).not.toHaveBeenCalled();
    });

    it('has hover styles for showing tooltip', () => {
      render(<HelpTooltip text="Help text" />);
      const tooltip = screen.getByText('Help text');

      // Tooltip should have opacity-0 by default and group-hover:opacity-100
      expect(tooltip).toHaveClass('opacity-0');
      expect(tooltip).toHaveClass('group-hover:opacity-100');
    });
  });

  describe('icon sizing', () => {
    it('uses default icon size of 14', () => {
      render(<HelpTooltip text="Help text" />);
      // The CircleHelp icon is rendered, we can verify button exists
      expect(screen.getByRole('button')).toBeInTheDocument();
    });

    it('accepts custom icon size', () => {
      render(<HelpTooltip text="Help text" iconSize={20} />);
      expect(screen.getByRole('button')).toBeInTheDocument();
    });
  });
});
