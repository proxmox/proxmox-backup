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
    queryMode: 'local',

    matchFieldWidth: false,
    listConfig: {
	minWidth: 170,
	maxWidth: 500,
	// below doesn't work :/
	minHeight: 30,
	emptyText: `<div class="x-grid-empty">${gettext('No namespaces accessible.')}</div>`,
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

    setDatastore: function(datastore) {
	let me = this;
	if (datastore ?? false) {
	    me.datastore = datastore;
	    me.store.getProxy().setUrl(`/api2/json/admin/datastore/${me.datastore}/namespace`);
	    if (me.isDisabled()) {
		me.setDisabled(false);
	    }
	    me.store.load();
	    me.validate();
	} else {
	    me.datastore = undefined;
	    me.setDisabled(true);
	}
    },

    initComponent: function() {
	let me = this;
	if (!me.datastore) {
	    me.disabled = true;
	}

	me.store = Ext.create('Ext.data.Store', {
	    model: 'pbs-namespaces',
	    autoLoad: !!me.datastore,
	    filters: (rec) => rec.data.ns !== '',
	    proxy: {
		type: 'proxmox',
		timeout: 30 * 1000,
		url: `/api2/json/admin/datastore/${me.datastore}/namespace`,
	    },
	});

	me.callParent();
    },
});
