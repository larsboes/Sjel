## Default Permission

Expose the iOS Keychain-backed Sjel device identity.

#### This default permission set includes the following:

- `allow-get-identity`
- `allow-signing`
- `allow-reset-identity`

## Permission Table

<table>
<tr>
<th>Identifier</th>
<th>Description</th>
</tr>


<tr>
<td>

`device-identity:allow-getIdentity`

</td>
<td>

Enables the getIdentity command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`device-identity:deny-getIdentity`

</td>
<td>

Denies the getIdentity command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`device-identity:allow-resetIdentity`

</td>
<td>

Enables the resetIdentity command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`device-identity:deny-resetIdentity`

</td>
<td>

Denies the resetIdentity command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`device-identity:allow-sign`

</td>
<td>

Enables the sign command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`device-identity:deny-sign`

</td>
<td>

Denies the sign command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`device-identity:allow-get-identity`

</td>
<td>

Read the device identity's public key. The private key remains in iOS Keychain.

</td>
</tr>

<tr>
<td>

`device-identity:allow-signing`

</td>
<td>

Sign request bytes with the Keychain-backed device key.

</td>
</tr>

<tr>
<td>

`device-identity:allow-reset-identity`

</td>
<td>

Replace the Keychain-backed device identity after explicit re-pairing.

</td>
</tr>
</table>
