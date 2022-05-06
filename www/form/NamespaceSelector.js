Ext.define('pbs-namespaces', {
    extend: 'Ext.data.Model',
    fields: [
	{
	    name: 'ns',
	},
	{
	    name: 'id', // fake as else the model messes with our value and/or display...
	    type: 'string',
	    calculate: data => data.ns === '' ? '/' : data.ns,
	},
    ],
    idProperty: 'id',
});

Ext.define('PBS.form.NamespaceSelector', {
    extend: 'Ext.form.field.ComboBox',
    alias: 'widget.pbsNamespaceSelector',

    allowBlank: true,
    autoSelect: true,
    valueField: 'ns',

    displayField: 'ns',
    emptyText: gettext('Root'),

    editable: true,
    anyMatch: true,
    forceSelection: true,

    matchFieldWidth: false,
    listConfig: {
	minWidth: 170,
	maxWidth: 500,
	// below doesn't work :/
	//minHeight: 30,
	//emptyText: gettext('No namespaces accesible.'),
    },

    triggers: {
	clear: {
	    cls: 'pmx-clear-trigger',
	    weight: -1,
	    hidden: true,
	    handler: function() {
		this.triggers.clear.setVisible(false);
		this.setValue('');
	    },
	},
    },

    listeners: {
	change: function(field, value) {
	    let canClear = value !== '';
	    field.triggers.clear.setVisible(canClear);
	},
    },

    initComponent: function() {
	let me = this;
	if (!me.datastore) {
	    console.error("no datastore passed");
	    return;
	}

	me.store = Ext.create('Ext.data.Store', {
	    model: 'pbs-namespaces',
	    autoLoad: true,
	    proxy: {
		type: 'proxmox',
		timeout: 30 * 1000,
		url: `/api2/json/admin/datastore/${me.datastore}/namespace`,
	    },
	});

	me.callParent();
    },
});

