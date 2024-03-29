window.addEventListener('DOMContentLoaded', (event) => {
    let activeSection = document.querySelector("a.current");
    if (activeSection) {
        // https://developer.mozilla.org/en-US/docs/Web/API/Element/scrollIntoView
        activeSection.scrollIntoView({ block: 'center' });
    }
});
