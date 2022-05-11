Ext.define('PBS.form.RemoteStoreSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsRemoteStoreSelector',

    queryMode: 'local',

    valueField: 'store',
    displayField: 'store',
    notFoundIsValid: true,

    matchFieldWidth: false,
    listConfig: {
	loadingText: gettext('Scanning...'),
	width: 350,
	columns: [
	    {
		header: gettext('Datastore'),
		sortable: true,
		dataIndex: 'store',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	    {
		header: gettext('Comment'),
		dataIndex: 'comment',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	],
    },

    doRawQuery: function() {
	// do nothing.
    },

    setRemote: function(remote) {
	let me = this;

	if (me.remote === remote) {
	    return;
	}

	me.remote = remote;

	let store = me.store;
	store.removeAll();

	if (me.remote) {
	    me.setDisabled(false);
	    if (!me.firstLoad) {
		me.clearValue();
	    }

	    store.proxy.url = '/api2/json/config/remote/' + encodeURIComponent(me.remote) + '/scan';
	    store.load();

	    me.firstLoad = false;
	} else {
	    me.setDisabled(true);
	    me.clearValue();
	}
    },

    initComponent: function() {
	let me = this;

	me.firstLoad = true;

	let store = Ext.create('Ext.data.Store', {
	    fields: ['store', 'comment'],
	    proxy: {
		type: 'proxmox',
		url: '/api2/json/config/remote/' + encodeURIComponent(me.remote) + '/scan',
	    },
	});

	store.sort('store', 'ASC');

	Ext.apply(me, {
	    store: store,
	});

	me.callParent();
    },
});

Ext.define('PBS.form.RemoteNamespaceSelector', {
    extend: 'Proxmox.form.ComboGrid',
    alias: 'widget.pbsRemoteNamespaceSelector',

    queryMode: 'local',

    valueField: 'ns',
    displayField: 'ns',
    emptyText: PBS.Utils.render_optional_namespace(''),
    notFoundIsValid: true,

    matchFieldWidth: false,
    listConfig: {
	loadingText: gettext('Scanning...'),
	width: 350,
	columns: [
	    {
		header: gettext('Namespace'),
		sortable: true,
		dataIndex: 'ns',
		renderer: PBS.Utils.render_optional_namespace, // FIXME proper root-aware renderer
		flex: 1,
	    },
	    {
		header: gettext('Comment'),
		dataIndex: 'comment',
		renderer: Ext.String.htmlEncode,
		flex: 1,
	    },
	],
    },

    doRawQuery: function() {
	// do nothing.
    },

    setRemote: function(remote) {
	let me = this;

	if (me.remote === remote) {
	    return;
	}

	me.remote = remote;

	let store = me.store;
	store.removeAll();

	me.setDisabled(true);
	me.clearValue();
    },

    setRemoteStore: function(remoteStore) {
	let me = this;

	if (me.remoteStore === remoteStore) {
	    return;
	}

	me.remoteStore = remoteStore;

	let store = me.store;
	store.removeAll();

	if (me.remote && me.remoteStore) {
	    me.setDisabled(false);
	    if (!me.firstLoad) {
		me.clearValue();
	    }

	    store.proxy.url = '/api2/json/config/remote/' + encodeURIComponent(me.remote) + '/scan/' + encodeURIComponent(me.remoteStore) + '/namespaces';
	    store.load();

	    me.firstLoad = false;
	} else {
	    me.setDisabled(true);
	    me.clearValue();
	}
    },

    initComponent: function() {
	let me = this;

	me.firstLoad = true;

	let store = Ext.create('Ext.data.Store', {
	    fields: ['ns', 'comment'],
	    proxy: {
		type: 'proxmox',
		url: '/api2/json/config/remote/' + encodeURIComponent(me.remote) + '/scan',
	    },
	});

	store.sort('store', 'ASC');

	Ext.apply(me, {
	    store: store,
	});

	me.callParent();
    },
});
