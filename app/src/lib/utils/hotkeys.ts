import { tinykeys } from 'tinykeys';

export enum KeyName {
	Space = ' ',
	Meta = 'Meta',
	Alt = 'Alt',
	Ctrl = 'Ctrl',
	Enter = 'Enter',
	Escape = 'Escape',
	Tab = 'Tab',
	Up = 'ArrowUp',
	Down = 'ArrowDown',
	Left = 'ArrowLeft',
	Right = 'ArrowRight',
	Delete = 'Backspace'
}

export function on(combo: string, callback: (event: KeyboardEvent) => void) {
	const comboContainsControlKeys =
		combo.includes(KeyName.Meta) || combo.includes(KeyName.Alt) || combo.includes(KeyName.Ctrl);

	return tinykeys(window, {
		[combo]: (event) => {
			const target = event.target as HTMLElement;
			const isInput = target.tagName === 'INPUT' || target.tagName === 'TEXTAREA';
			if (isInput && !comboContainsControlKeys) return;

			event.preventDefault();
			event.stopPropagation();

			callback(event);
		}
	});
}
