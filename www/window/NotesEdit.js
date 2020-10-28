Ext.define('PBS.window.NotesEdit', {
    extend: 'Proxmox.window.Edit',
    mixins: ['Proxmox.Mixin.CBind'],

    title: gettext('Notes'),

    width: 600,
    height: '400px',
    resizable: true,
    layout: 'fit',

    autoLoad: true,

    defaultButton: undefined,

    notesFieldName: 'notes',

    setValues: function(values) {
	let me = this;
	if (typeof values === "string") {
	    let v = values;
	    values = {};
	    values[me.notesFieldName] = v;
	}
	me.callParent([values]);
    },

    items: {
	xtype: 'textarea',
	name: 'notes',
	cbind: {
	    name: '{notesFieldName}',
	},
	height: '100%',
	value: '',
	hideLabel: true,
    },
});
