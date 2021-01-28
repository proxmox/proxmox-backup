Ext.define('PBS.TapeManagement.RetentionStore', {
    extend: 'Ext.data.Store',
    alias: 'store.retentionCalendarEventStore',

    field: ['value', 'text'],
    data: [
	{ value: 'overwrite', text: gettext('Overwrite') },
	{ value: 'keep', text: gettext('Keep') },
	{ value: '120 minutes', text: Ext.String.format(gettext("{0} minutes"), 120) },
	{ value: '12 hours', text: Ext.String.format(gettext("{0} hours"), 12) },
	{ value: '7 days', text: Ext.String.format(gettext("{0} days"), 7) },
	{ value: '4 weeks', text: Ext.String.format(gettext("{0} weeks"), 4) },
	{ value: '6 months', text: Ext.String.format(gettext("{0} months"), 6) },
	{ value: '2 years', text: Ext.String.format(gettext("{0} years"), 2) },
    ],
});

Ext.define('PBS.TapeManagement.RetentionSelector', {
    extend: 'PBS.form.CalendarEvent',
    alias: 'widget.pbsRetentionSelector',

    store: {
	type: 'retentionCalendarEventStore',
    },
});

