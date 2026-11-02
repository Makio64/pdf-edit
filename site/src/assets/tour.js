// Home page: the app's four screens under the title, one at a time, with a dot per step.
// It always moves on by itself. A click on a dot shows that step and lets it stay a little longer;
// only keyboard focus on a dot holds it, so that a screen reader can read it. With reduced motion
// the screens change without fading (the stylesheet turns transitions off).
const tour = document.querySelector('[data-tour]');

if (tour) {
  const slides = [...tour.querySelectorAll('.tour-slide')];
  const dots = [...tour.querySelectorAll('.tour-dot')];
  const caption = tour.querySelector('.tour-caption');
  const STEP_MS = 4000;
  const AFTER_CLICK_MS = 9000;
  let current = 0;
  let timer = null;
  let held = false;

  const show = (index) => {
    current = (index + slides.length) % slides.length;
    slides.forEach((slide, i) => {
      slide.classList.toggle('is-active', i === current);
      slide.setAttribute('aria-hidden', String(i !== current));
    });
    dots.forEach((dot, i) => {
      dot.classList.toggle('is-active', i === current);
      if (i === current) {
        dot.setAttribute('aria-current', 'step');
      } else {
        dot.removeAttribute('aria-current');
      }
    });
    caption.textContent = dots[current].dataset.caption;
  };
  const schedule = (ms = STEP_MS) => {
    clearTimeout(timer);
    timer = null;
    if (!held && !document.hidden) {
      timer = setTimeout(() => {
        show(current + 1);
        schedule();
      }, ms);
    }
  };

  dots.forEach((dot, i) =>
    dot.addEventListener('click', () => {
      show(i);
      schedule(AFTER_CLICK_MS);
    })
  );
  tour.addEventListener('focusin', (event) => {
    if (event.target.matches(':focus-visible')) {
      held = true;
      // Read aloud only for the person moving through the steps, not on every automatic change.
      caption.setAttribute('aria-live', 'polite');
      schedule();
    }
  });
  tour.addEventListener('focusout', () => {
    if (held) {
      held = false;
      caption.removeAttribute('aria-live');
      schedule();
    }
  });
  document.addEventListener('visibilitychange', () => schedule());

  tour.dataset.ready = '';
  schedule();
}
