Ext.define('PBS.TapeManagement', {
    extend: 'Ext.tab.Panel',
    alias: 'widget.pbsTapeManagement',

    title: gettext('Tape Backup'),

    border: true,
    defaults: { border: false },

    html: "Experimental tape backup GUI.",
});
